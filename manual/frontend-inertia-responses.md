# Inertia Responses

Inertia responses are how a Suprnova handler ships state to a Svelte / React /
Vue page component. Every handler that renders an Inertia page returns one,
built either through the [`inertia_response!`](#the-inertia_response-macro)
macro (for typed, compile-time-checked eager props) or the
[`InertiaResponse`](#the-inertiaresponse-builder) builder (for everything
else - lazy props, deferred props, merge, once, scroll, flash). This
chapter covers the response surface end-to-end: the macro, the builder, the
v3 protocol features (partial reloads, history encryption, version
detection), shared data via `App::inertia_share*`, and the flash bag carried
across redirects.

If you haven't picked a frontend yet, [Frontend Overview](frontend.md) and
[Page Components](frontend-pages.md) come first; this chapter assumes the
SPA bridge is wired and focuses on what your handler returns.

## The `inertia_response!` macro

The macro is the shortest path from a handler to a typed eager page. It
takes the current request, a component name, and a props expression:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Three things to know:

- **The leading `&req` is required.** The macro reads `X-Inertia` headers,
  the URL, and the partial-reload filtering headers off the request, so it
  needs the request value (or a reference). Without it, partial reloads
  would silently break.
- **Component existence is checked at compile time.** The macro looks for
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; if no file
  matches, the build fails with a "did you mean…?" suggestion sourced from
  the actual filenames on disk. Nested paths work the same way -
  `inertia_response!(&req, "Admin/Dashboard", …)` resolves
  `frontend/src/pages/Admin/Dashboard.svelte` (or your frontend's
  extension).
- **The macro expands to an `await`ed `Result`.** Your handler must
  return [`Response`](error-model.md) (which is
  `Result<HttpResponse, HttpResponse>`) or another type that absorbs
  `FrameworkError` through `?` / `From`. Failures during prop
  serialization or response building are returned as `Err`, not panics.

### JSON-style props

For prototyping and tiny pages you can skip the typed struct:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

The macro still validates the component file. The trade-off is that you
lose the typed-prop chain - no `#[derive(InertiaProps)]`, no automatic
TypeScript generation, no compile-time check that the frontend's
expected shape matches.

### Optional config override

The macro accepts an optional trailing `InertiaConfig` for per-response
overrides (different SSR settings, a custom default title for one page):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

Most apps register a single config at boot via [`Inertia::install`](#bootstrap-inertia-install)
and never touch this argument - the installed config is already what
every response starts from. Pass one here only to override the installed
config for a single page.

## `#[derive(InertiaProps)]`

`InertiaProps` emits a `Serialize` impl whose key names match your field
names. It exists so the typed-props path stays terse and so the
TypeScript generator (`suprnova generate-types`) has a marker to find:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Nested types compose normally - fields can be `Vec<T>`, `Option<T>`,
nested structs, anything `Serialize`-able. The nested types themselves
don't have to derive `InertiaProps`; they just need `Serialize`. Use
`#[derive(InertiaProps)]` on the *top-level* props struct and you get
the automatic TypeScript surface (see [TypeScript Types](frontend-typescript-types.md))
for the whole tree.

## The `InertiaResponse` builder

The macro covers eager typed props. Anything else - lazy, optional, deferred,
mergeable, cached-on-client, flash, history-encryption overrides - uses
the builder directly:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: closure runs only when the prop will actually be sent
        // (initial visit, or partial reload that requests this key).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: never sent on initial visits; the client must
        // explicitly ask for the key via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: skipped on the initial render; the client issues a
        // follow-up XHR and the closure runs then.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: append-into-existing on partial reloads ("load more").
        .merge("rows", next_page().await?)
        // Once: cached client-side across navigations; resolver skipped
        // on subsequent visits unless server forces refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: one-shot toast; appears under `page.flash`, not `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Method | Purpose | Maps to Laravel |
|---|---|---|
| `.with(k, v)` | Eager prop, honours partial-reload filtering | typed prop |
| `.always(k, v)` | Eager prop, ignores partial-reload filters | `Inertia::always(…)` |
| `.lazy(k, ‖)` | Resolver runs only when prop will be sent | `fn () => …` closure |
| `.optional(k, ‖)` | Never on initial visit; must be requested explicitly | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Initial-visit-skipped; follow-up XHR triggers resolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine with existing client state on partial reloads | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Client caches across navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (via `Inertia::paginate`) | Infinite-scroll pagination | `Inertia::scroll(…)` |
| `.flash(k, v)` | One-shot value under `page.flash` (not `props`) | `session()->flash(…)` |
| `.title(…)` | Default `<title>` for the HTML shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Per-response history encryption | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force history key rotation on **this** page | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Keep `#fragment` after Inertia visit | `Inertia::preserveFragment()` |

Eager builder methods have `try_*` siblings (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_flash`) that return
`Result<Self, FrameworkError>` when a value's `Serialize` impl might
fail at runtime - the infallible methods convert the panic into a 500
via [the panic boundary](error-model.md), so reach for `try_*` when
you'd rather handle the failure explicitly.

`.clear_history()` marks the response you are building. A logout handler
redirects, and the browser discards the redirect's response - so the login
page, not the logout response, is the one that has to carry the flag.
`App::clear_history()` is the fix for that case - it's a free function, not
a builder method, so it isn't in the table above. It flashes a one-shot
session flag that the next Inertia page object turns into
`clearHistory: true`. It needs a session scope, and it survives exactly
one hop.

Call it **after** `Auth::logout()` / `Auth::logout_and_invalidate()`, not
before - invalidation flushes the whole session, and the flag lives in
that session, so flashing it first only gets erased by the flush:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Merge strategies and infinite scroll

`.merge` (append), `.merge_prepend`, and `.deep_merge` cover the common
"load more" cases. To diff-merge - update rows the client already holds
instead of duplicating them - reach for `.merge_with` with an explicit
`MergeStrategy` carrying a `match_on` key:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` names the field the client dedupes on (emitted to the page
object as `matchPropsOn`), so a refetch that overlaps the current window
replaces matching rows in place rather than appending copies. `Prepend`
and `Deep` take the same `match_on`.

Infinite scroll is the same machinery with pagination metadata attached.
`.scroll` / `.scroll_with` - or `.paginate`, which adapts a
`LengthAwarePaginator` or `CursorPaginator` directly - emit `scrollProps`
next to the data, and the client's `<InfiniteScroll>` component drives the
next/previous fetches:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

The framework reads the merge direction from the
`X-Inertia-Infinite-Scroll-Merge-Intent` request header the client sends
(`append` when scrolling down, `prepend` when scrolling up). On a fresh
visit - no intent header - `scrollProps["posts"].reset` is `true`, so the
client clears its accumulator before rendering the first window.

## Partial reloads

The Inertia 3 client can request a subset of a page's props (or a
superset by including an Optional or Defer key). The protocol uses
three request headers:

| Header | Meaning |
|---|---|
| `X-Inertia-Partial-Component` | The component being partial-reloaded - must match the response's component for filtering to apply. |
| `X-Inertia-Partial-Data` | Whitelist: comma-separated prop keys to include. |
| `X-Inertia-Partial-Except` | Blacklist: comma-separated prop keys to exclude. Wins over `Partial-Data` on key collision. |

Filtering rules:

- `Eager`, `Lazy`, `Merge`, `Once`, `Scroll` props follow whitelist /
  blacklist semantics.
- `Always` props are sent regardless.
- `Optional` and `Defer` props are never on a standard visit and only
  appear on a matching partial reload that explicitly lists the key.

The handler doesn't have to do anything special - register every prop
through the builder, and the framework consults the headers when
serializing the page object.

A `once` prop's client-side cache is honoured only on a **full** Inertia
visit. On a partial reload that names the key
(`router.reload({ only: ['stats'] })`), the resolver runs and the value is
sent - the client asked precisely because it wants a fresh one, and
honouring its stale-cache claim there would return nothing at all for the
key it asked for.

## Shared data via `App::inertia_share*`

Some props are the same on every Inertia page - auth state, the CSRF
token, the current locale, app-wide flags. Register them once at
bootstrap and they merge into every response:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materialized once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolved per response (skipped by partial reloads that
    // exclude the key).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cached on the client across navigations - `share_once` runs on
    // the first page that needs it, then the client skips re-resolution
    // via `X-Inertia-Except-Once-Props` until the cache key changes.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

For per-request shared data (the authenticated user, request-scoped
flags), implement [`InertiaSharedData`](#per-request-shared-data) and
register the singleton - the framework calls `share(&req)` on every
Inertia response and merges the result.

### Precedence on key collision

When the same key appears in more than one layer, later writes win:

1. Static registry (`App::inertia_share` / `App::inertia_share_lazy`)
2. Per-request trait provider (`InertiaSharedData::share`)
3. Per-response builder methods (`.with`, `.lazy`, etc.)

This lets a handler override a globally-shared default for one page
without having to unregister anything.

### Per-request shared data

The trait runs once per Inertia response with access to the request.
Implementations need `async_trait` (re-exported as `suprnova::__async_trait`)
and `IndexMap` (re-exported as `suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

## Flash and redirects

Flash data is one-shot state that should appear on the next render and
disappear after - toast messages, "just created" IDs, validation summaries.
Suprnova surfaces it under `page.flash` on every Inertia response. There
are three writers:

```rust
// 1. Push into the current request's flash bag.
App::flash("toast", "Saved");

// 2. Attach to a specific response (same effect on this response only).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carry across a redirect via the Redirect facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

The `Redirect::with(key, value)` form is the cross-handler path: the
value lands in the session under `_flash.new.*`, the next request's
[`SessionMiddleware`](csrf.md) ages it into `_flash.old.*`, and the
destination's `InertiaResponse` surfaces it under `page.flash`.

Same-request flash (the task-local bag) wins over inherited session
flash on key collision, so a destination handler can override an
inbound value just by re-flashing the key.

Internal session keys (anything prefixed `_`) are filtered out of
`page.flash` - `_old_input` for form repopulation and `_inertia.*`
protocol flags don't leak to the client.

### Redirect helpers

`Redirect` is the full Laravel surface:

```rust
Redirect::to("/dashboard")                       // 302 to a path
Redirect::route("posts.show").with("id", "42")   // named route, route params
Redirect::back("/")                              // session-recorded previous URL
Redirect::refresh()                              // same URL, fresh GET
Redirect::guest(&req, "/login")                  // stashes intended URL
Redirect::intended("/dashboard")                 // pops the stashed URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signed URL
Redirect::to("/posts/42").preserve_fragment()    // keep #frag across visit
```

All `Redirect` variants accept `.with(k, v)`, `.with_input(map)`,
`.with_errors(map)`, `.with_errors_bag(name, map)`, `.cookie(c)`,
`.header(k, v)`, `.permanent()`, `.status(303)`, etc. The full chain
mirrors Laravel's `RedirectResponse`.

For non-GET Inertia visits, the framework auto-converts the response to
`303 See Other` when [`Inertia303Middleware`](#bootstrap-inertia-install)
is installed, so the browser issues a clean follow-up GET instead of
re-submitting the original PUT/PATCH/DELETE to the redirect target.

To send the visitor **out** of the Inertia app - a payment provider, an
OAuth authorize endpoint, a hosted billing portal - use `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

An Inertia XHR gets `409` + `X-Inertia-Location` (the client runs
`window.location = url`); a hard navigation gets a plain `302` + `Location`.
The bare `InertiaResponse::location(url)` always returns the 409 form - use
it only where the request is already known to be an Inertia visit, because
a browser that follows a `409` with no `Location` header has nowhere to go.

## Version detection

Inertia versions the asset manifest so a long-lived client doesn't try
to mount a page from yesterday's bundle against today's server. When
the client's `X-Inertia-Version` header doesn't match the server's
configured version, [`InertiaVersionMiddleware`](#bootstrap-inertia-install)
responds with `409 Conflict` and an `X-Inertia-Location` header naming
the new URL - the Inertia client picks that up and does a full page
reload, picking up the new bundle.

The bounce re-flashes the session first. The client answers a 409 with a
full-page GET, and that GET is a fresh request - without the re-flash, a
validation error or success message flashed by the previous request is aged
away before the destination page can read it, and the user loses their error
message purely because a deploy landed mid-submit. This needs
`SessionMiddleware` registered ahead of the version middleware.

By default you set nothing: `InertiaConfig` hashes your Vite build
manifest (`manifest_path`, default `public/assets/.vite/manifest.json`)
and uses the first 16 bytes of its SHA-256, hex-encoded. The manifest is
the one file that changes on every build and on no other occasion, so
the version bumps itself. When there is no manifest to read - local
development, where Vite serves from memory - it falls back to the static
string `"1.0"` and logs at `debug`.

Override it when you want something else:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

The manifest is read on every version check, which is what Laravel's
`hash_file` does too - a few KB out of the page cache, and a rebuild is
picked up immediately. If you have measured that and want it gone,
resolve once at boot:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

For async or fallible version resolution (e.g. read a manifest hash
from S3), do the read once at boot and pass the cached `String` to
`.version(...)`.

## Bootstrap: `Inertia::install`

Most apps install the three protocol middlewares in one call:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …other shared data, routes, etc.
    Ok(())
}
```

`Inertia::install` returns `Result` and, in order:

1. Fails closed if `cfg` resolves to production mode (`development ==
   false` - the default whenever `APP_ENV=production`) but no Vite
   manifest can be loaded from `cfg.manifest_path`. This is the CFG-01
   guard: a production boot with an unbuilt frontend errors loudly
   instead of silently falling back to a legacy hardcoded asset path.
2. Registers `InertiaHeadersMiddleware` - sets `Vary: X-Inertia` on every
   response and turns an empty `200` on an Inertia visit into a `303` back.
3. Registers `InertiaVersionMiddleware` - emits the `409` + `X-Inertia-Location`
   when client and server disagree on the asset version.
4. Registers `Inertia303Middleware` - upgrades `302` to `303` on non-GET
   Inertia redirects.

Order matters: the headers middleware is registered first, so it is the
outermost and sees every response - including the `409` the version
middleware returns before the handler ever runs.

`install` also **retains the config**. Every `InertiaResponse` built
afterwards starts from it, so `.frontend(...)`, `.version(...)`,
`.default_title(...)`, `.ssr(...)` and `.encrypt_history(...)` set here
reach every page without a handler passing anything. A handler that wants
different settings for one page still overrides with `.with_config(...)`;
an app that never calls `Inertia::install` gets `InertiaConfig::default()`;
and calling `install` again replaces the retained config.

`.with_config(...)` replaces the config wholesale, `version` included.
`InertiaVersionMiddleware` still resolves the version `Inertia::install`
was given, so a config here that doesn't carry the same `.version(...)`
makes the page object advertise a version the middleware will bounce - the
client takes one extra full page load after visiting that page. Set
`.version(...)` on the override to match.

Register `SessionMiddleware` **ahead of** `Inertia::install` if you use
flash data. The version middleware re-flashes the session before bouncing
the client, so a flashed error survives the follow-up full-page GET; it
can only do that inside a session scope.

Skip the call only if you genuinely don't want one of these middlewares
(rare; all three close real failure modes - cache poisoning across the two
representations of a URL, silent stale-bundle, and form-replay-on-redirect).

## Server-driven `<head>` elements

Inertia 3.5 added a client option for letting the server decide what goes in
`<head>` - useful when meta tags depend on the record you just loaded, and you
don't want the title and OG tags to live in two places.

This needs no framework support. The client reads the elements from an
**ordinary prop**, so any handler can supply them:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

Opt in on the client:

```js
createInertiaApp({
  serverHead: true,        // reads the `head` prop
  // serverHead: 'meta',   // or read a differently-named prop
  // serverHead: (page) => [...],  // or compute from the whole page
})
```

Each string is an HTML element. The client stamps a `data-inertia` attribute on
anything that lacks one so it can diff head elements across navigations; supply
your own `data-inertia="og-title"` when you want stable identity rather than
positional matching.

Escape anything interpolated from user data - these strings are injected as
HTML, so the usual rules apply.

## SSR

Suprnova talks to an out-of-process SSR worker - typically the
`@inertiajs/{svelte,react,vue}/server` `createServer()` bundle run
under Node / Bun / Deno - over HTTP loopback. Enable it on the config you
hand to [`Inertia::install`](#bootstrap-inertia-install) - that config is
what every response starts from, so there is nothing to plumb through
your handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR is off by default, and it is a property of the config: on for every
response built from the installed config, off for any response that
overrides with a `.with_config(...)` which doesn't set it. When enabled,
the framework posts the page
object to `<url>/render` and inlines `{ head, body }` in the HTML
shell. On worker error or timeout the response falls back to CSR
(an empty `<div id="app">` the client hydrates) and the
`on_ssr_error(...)` hook fires; flip `ssr_throw_on_error(true)` in CI
to make those failures hard 500s instead.

Boot the worker separately - `suprnova ssr:start` is the standard
runner once your project ships an SSR entry.

## Configuration

Inertia behaviour is configured programmatically via `InertiaConfig`, and
the config you hand to [`Inertia::install`](#bootstrap-inertia-install) is
the one every response starts from. The one env var the framework reads
directly is `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), and it only
supplies the default entry-point filename and page-component extensions
when the config doesn't say - an explicit `.frontend(Frontend::React)` on
the installed config wins, and is what `suprnova new --frontend react`
scaffolds. Everything else is builder-shaped:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
```

Frontend-specific defaults:

| Frontend | Default entry point | Page extensions |
|---|---|---|
| Svelte (default) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### The `url` field

`page.url` is the path **and** query string of the request
(`/users?page=2&sort=name`). The client writes it into `history.state`, so
it is what back/forward navigation and `router.reload()` replay - drop the
query and every paginated or filtered page silently resets to page one.
`InertiaVersionMiddleware` derives its `X-Inertia-Location` from the
request's path and query too, so by default a 409 asset-version bounce
lands the browser on exactly the URL the page object named.

Override the derivation with `url_resolver` when the URL the client should
record differs from the one that arrived - a locale prefix the SPA doesn't
route on, or a path a reverse proxy rewrote:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

The resolver reads the request through `InertiaRequestExt`, and applies to
every response built from the config you pass to
[`Inertia::install`](#bootstrap-inertia-install) - the usual place for a
resolver that should apply app-wide. Override it for a single response
with `InertiaResponse::with_config(cfg)`. A resolver changes `page.url`
only. The 409 bounce keeps naming the URL that actually arrived - that is
the URL the browser has to fetch - so with a resolver in place the two
deliberately differ.

The Vite manifest at `manifest_path` is loaded lazily on first request
and cached for the process lifetime - every response built from the
installed config shares that one cache, so the file is read and parsed
once. When it's missing, production asset tags fall back to a hardcoded
legacy path and a `tracing::warn!` fires so the gap surfaces in logs.

### Why Suprnova diverges

Laravel's Inertia adapter has a single global "shared data"
registry plus a per-request `Inertia::share($k, $v)` call. PHP's
request-per-process model makes this safe: a fresh process per request
means no leakage between concurrent visitors.

Rust's process model is the opposite - one process serves many
concurrent requests across many threads. So the registry lives on
the [container](container.md) (task-local → thread-local → global),
not in process-global statics. `App::inertia_share*` writes to the
active container's `InertiaRegistry`, which gives tests using
`TestContainer::fake()` clean isolation without having to unregister
anything. Same surface as Laravel; different machinery underneath
because the runtime is different.

Five other Rust-shaped choices worth flagging:

- **Lazy-prop resolvers run concurrently**, capped by
  `max_concurrent_resolvers` (default 16). A page with twelve lazy
  props issues twelve parallel queries inside one Tokio task - that's
  what we built the framework on top of Tokio for. Tune the cap if a
  page has many lazy props each hitting an external service.
- **The compile-time component check** isn't a Laravel feature at all,
  because PHP can't see your frontend files at compile time. Suprnova
  does, so a typo in `inertia_response!("Dashbaord", …)` fails the
  build with a "did you mean Dashboard?" suggestion instead of
  surfacing as a runtime "component not found" later.
- **An empty `200` on an Inertia visit becomes a `303`, not a `302`.**
  Laravel's `onEmptyResponse` returns `redirect()->back()` (a 302) and
  relies on its later `302 → 303` conversion for PUT/PATCH/DELETE only. A
  substituted redirect is never a continuation of the original method - the
  client has to issue a GET - so Suprnova says `303` directly instead of
  leaving GET visits on a 302 the client would follow with the original
  verb.
- **`Inertia::location($url)` is two methods here, not one.** `location(url)`
  keeps Laravel's always-`409` contract - it predates the request-aware
  form and pinned-tag consumers depend on that shape not changing.
  `location_for(&req, url)` is the newer, request-aware form: `409` for an
  Inertia XHR, plain `302` for a hard navigation. Reach for `location_for`
  in new code.
- **`Inertia::clearHistory()` is two methods here, not one, either.**
  `.clear_history()` on the builder marks a single response; `App::clear_history()`
  flashes the flag into the session so it survives a redirect. Laravel gets
  away with one method because it's already session-backed - Suprnova
  keeps the response-local form as the default (no session dependency) and
  makes the cross-redirect case an explicit opt-in instead.

## Next

- [Page Components](frontend-pages.md) - how the frontend resolves a
  component name to a Svelte / React / Vue module
- [TypeScript Types](frontend-typescript-types.md) - `suprnova generate-types`
  emits TS definitions from your `#[derive(InertiaProps)]` structs
- [Data Objects](data.md) - `#[derive(Data)]` for DTOs with per-field
  include/allowlist gating that composes with partial reloads
- [Error Model](error-model.md) - how `Response`, the panic boundary,
  and `FrameworkError` thread through Inertia responses
- [Container](container.md) - the lookup model behind
  `App::inertia_share*` and `InertiaSharedData`
