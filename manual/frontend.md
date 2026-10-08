# Frontend Overview

Suprnova bridges Rust handlers to a single-page frontend via
[Inertia.js](https://inertiajs.com/) 3.4.0. You write controllers in Rust
and pages in Svelte, React, or Vue; the framework moves typed props
between them without a separate HTTP API in the middle.

## Three first-class starters

`suprnova new <name>` scaffolds a working project. The `--frontend` flag
picks the SPA layer:

```bash
suprnova new my-app                       # Svelte 5 (default)
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

All three scaffolds share the same stack:

| Layer | Version |
|---|---|
| Inertia client adapter | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| Build tool | Vite 8 |
| Styling | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | strict mode |

The choice is per-project. There is no "primary" framework on the
server side - `inertia_response!` resolves whichever extension your
chosen scaffold uses (`.svelte`, `.tsx`, `.vue`), and `App::inertia_share`,
partial reloads, and TypeScript prop generation all behave identically
across the three.

## Architecture

```
                       Browser
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  typed props from Rust struct       |
   |   +-------v-------------------------------+     |
   |   |        Inertia client adapter         |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON on XHR, HTML on first load)
                          v
   +-------------------------------------------------+
   |                  Suprnova server                |
   |   +------------------------------------------+  |
   |   |          Controllers / handlers          |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

The first request returns an HTML shell with the initial page object as
JSON in a `<script type="application/json" data-page="app">` element, the
sibling just before the empty `<div id="app">` mount node - where the
Inertia 3 client reads it. Subsequent visits go through `<Link>` /
`router.visit`, send `X-Inertia: true`, and get back a JSON page object -
the adapter swaps the component without a full reload.

## A complete page round-trip

The controller defines its props as a Rust struct, derives
`InertiaProps`, and hands the value to the `inertia_response!` macro:

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

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

The props are sent under the names serde's attributes give them:
`#[serde(rename_all = "camelCase")]` on the struct sends a `current_user`
field as `currentUser`, and `#[serde(rename = "..")]`, `#[serde(skip)]` and
`#[serde(skip_serializing)]` work on a field. Props are only serialized, so
the derive reads the serialize side, and leaves a `deserialize` half and
`skip_deserializing` to a `Deserialize` derive on the same struct. It
refuses any other serde attribute, and two props under one key.

A few things the macro does for you. First, it validates at compile
time that the page component file actually exists under
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` - typos surface as a
build error, not a 404 in the browser. Second, it serializes the
`HomeProps` struct, unfolds it into one prop per top-level key so
partial reloads can filter, and resolves any lazy or deferred props
against `&req` before returning. The macro evaluates to a
`Result<HttpResponse, FrameworkError>`, which the `Response` return type
accepts directly.

The matching Svelte page (the default scaffold):

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

For the React and Vue equivalents see [Page Components](frontend-pages.md).

## Generating TypeScript types

Every `#[derive(InertiaProps)]` struct in your `src/` becomes a
TypeScript interface in `frontend/src/types/inertia-props.ts`:

```bash
suprnova generate-types
```

Pass `--routes` and the same command also emits
`frontend/src/types/routes.ts` - type-safe URL + method pairs scraped
from your `routes!` macro that work directly with Inertia v2+ APIs. The full type-mapping table and
route-helper shape live in [TypeScript Types](frontend-typescript-types.md).

## Shared data

Anything that should appear on every page (the authenticated user, the
current locale, app metadata) is registered once at boot and merged into
every Inertia response:

```rust
// In bootstrap.rs
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// Async / per-request shared data goes through the trait.
App::register_inertia_shared(Arc::new(AppSharedData));
```

The flavours, in order of precedence (later wins at the same key):

| API | When the value materializes |
|---|---|
| `App::inertia_share(k, v)` | Sync, set once at boot |
| `App::inertia_share_lazy(k, \|\| async { ... })` | Per response, recomputed |
| `App::inertia_share_once(k, \|\| async { ... })` | Per response, then client-cached |
| `App::inertia_registry().share_provider(provider)` | Per response, expanded with the page and the request; any number |
| `App::register_inertia_shared(Arc::new(impl))` | Per request, sees `&req` |

Per-page props attached on the response builder always overwrite shared
data at the same key.

The `Inertia` facade takes every form Laravel's `Inertia::share` does - a
key and a value, a map, a `#[derive(Data)]` object or a provider - and reads
shared data back with `get_shared`:

```rust
use suprnova::Inertia;

Inertia::share("appName", "Suprnova")?;
Inertia::share("user.locale", "es")?;              // nests: user = { locale: "es" }
Inertia::share_many([("plan", "pro"), ("region", "eu")])?;
Inertia::share_data(SiteMeta { name: "Suprnova".into(), build: 7 })?;
Inertia::share_provider(TenantShare);              // a ProvidesInertiaProperties value

Inertia::get_shared("user.locale", serde_json::Value::Null); // "es"
Inertia::get_shared("missing", 7);                            // 7
Inertia::get_shared_all();                                    // every shared value, nested
```

A dotted key nests when it is shared, as Laravel's `Arr::set` does, so a
later `share("user", ...)` replaces the whole `user` object, child included.
`App::inertia_share` shares the same way. A Data object shares its eager
fields; its lazy fields stay out, since a shared prop has no `?include=`
gate. `share_provider` takes a `ProvidesInertiaProperties` value, the same
registration as `InertiaRegistry::share_provider`: any number of providers,
each expanded once per render with a `RenderContext` of the request and the
component, and `App::flush_inertia_shared` clears them with the other shares.
`App::register_inertia_shared` is the one-slot async provider
(`InertiaSharedData`) and stays beside it. `get_shared` reads what is
registered without resolving it, so a lazy share reads as the default.

The framework ships `RootShare`, a provider that gives every page the
public root as the `root` prop: the empty string at the host root,
`/billing` behind a reverse proxy that serves the application under
`/billing`. A page that builds its URLs from it, such as
`` form.post(`${root}/login`) ``, runs from one build at `/` and under a
prefix. An application registers one provider, so `RootShare` carries
another one and shares its props too:

```rust
App::register_inertia_shared(Arc::new(RootShare::around(Arc::new(LocaleShare))));
```

A scaffolded application registers it this way, builds its pages' URLs
from `root`, and builds its frontend with Vite's relative `base: './'`, so
code-split chunks load from wherever the entry script was served. For the
proxy setup, see [Serving under a path prefix](deployment.md#serving-under-a-path-prefix).

## Partial reloads and lazy props

The same `InertiaResponse` builder exposes Inertia v3's full prop
toolkit - eager, lazy, optional, deferred, merge, once - and Suprnova
honors the v3 partial-reload headers (`X-Inertia-Partial-Data`,
`X-Inertia-Partial-Except`, `X-Inertia-Reset`,
`X-Inertia-Except-Once-Props`) automatically. The example below
attaches three props with different evaluation rules:

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` covers the eager-props case; everything past that
goes through the builder. The full surface - `optional`, `merge`,
`once`, `scroll`, `flash`, `paginate`, SSR, version mismatch, history
encryption - is documented in
[Inertia Responses](frontend-inertia-responses.md).

## Bootstrap

A scaffolded app installs the four protocol-critical middlewares in one
call inside `bootstrap.rs`:

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` returns `Result` - it fails closed if `InertiaConfig` resolves to
production mode (the default under `APP_ENV=production`) but no Vite
manifest can be found, rather than silently falling back to a legacy
asset path. See [Development vs production](#development-vs-production)
below.

That registers, in order: `InertiaHeadersMiddleware` (sets `Vary: X-Inertia`
on every response and turns an empty `200` on an Inertia visit into a
redirect back), `InertiaVersionMiddleware` (emits 409 + `X-Inertia-Location` on
asset-version mismatch so stale clients reload), `Inertia303Middleware`
(rewrites 302 → 303 on non-GET Inertia visits so the follow-up is
unambiguously a GET), and `InertiaValidationRedirectMiddleware` (turns a
`422` on an Inertia visit into a `303` back to the form page with the
errors flashed). `InertiaVersionMiddleware` and `Inertia303Middleware`
used to require separate registration; `Inertia::install` makes all four
the default. See [Inertia Responses](frontend-inertia-responses.md#bootstrap-inertia-install)
for the full registration order and what each middleware closes.

## Development vs production

In development, the Vite dev server runs alongside the backend and
serves HMR-enabled assets:

```bash
suprnova serve
```

This boots the Rust server and `vite` together. The HTML shell loads
modules from `http://localhost:5765`.

For production, build the frontend once and point the backend at the
hashed manifest under `public/assets/`:

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` derives production vs. development mode from
`APP_ENV` (via `Environment::detect().is_production()`) - `APP_ENV=production`
is what makes the HTML shell load built assets instead of the Vite dev
server. `Inertia::install` then fails boot loudly if it can't find a
manifest to back that decision, rather than silently falling back to a
stale hardcoded path.

Suprnova reads `public/assets/.vite/manifest.json` to resolve hashed
entry points plus any transitive imports for `modulepreload`. SSR is
optional - opt in by pointing `InertiaConfig::ssr(...)` at a running
`@inertiajs/{vue3,react,svelte}/server` worker. `suprnova new` scaffolds
an SSR entry point and build script for every starter, and the
`ssr:start`, `ssr:stop`, and `ssr:check` commands of the application
binary and of the `suprnova` CLI run, stop, and check the worker;
see [Inertia Responses](frontend-inertia-responses.md#ssr) for the full
setup, including the bundle-existence check and CSR fallback behavior.

### Why Suprnova diverges

Three intentional departures from how a typical Inertia setup looks
elsewhere:

- **Compile-time component validation.** The `inertia_response!` macro
  walks `frontend/src/pages/` at build time and refuses to expand if
  the component file is missing, suggesting the closest match. You
  cannot ship a controller that points at a deleted page.
- **Typed props as the source of truth.** Page props are Rust structs
  with `#[derive(InertiaProps)]`. `suprnova generate-types` reads them
  and writes TypeScript interfaces - the frontend types are derived
  from the backend, not maintained in parallel.
- **Svelte as the default.** Inertia's documentation reaches for Vue and
  React first; the Suprnova scaffolder defaults to Svelte 5 (runes-on).
  React 19 and Vue 3.5 are first-class, not afterthoughts - same
  protocol, same prop pipeline, same generator output.

## Working with a coding assistant

A coding assistant that learned Inertia from Laravel projects carries
assumptions that are wrong here. Laravel's own guideline set for assistants
(the Inertia section of Laravel Boost) covers this ground for a PHP project;
this is the Suprnova version. Paste it, or point the assistant at this
chapter, before it touches a page:

- **Pages live in `frontend/src/pages/`**, one component per file. A handler
  renders one with `inertia_response!` or `InertiaResponse::new`, never with
  a server template. The first visit's document is `templates/app.html`, the
  root template, and the page is never written into it.
- **Props are typed on the server.** Each page has a `#[derive(InertiaProps)]`
  struct, and `suprnova generate-types` (or `suprnova serve`, which runs it
  on change) writes `frontend/src/types/inertia-props.ts`. After a Rust field
  changes, regenerate and let the TypeScript compiler name every component to
  update. The generated file is never edited by hand.
- **Every Inertia 3 feature is available**: deferred props, infinite scroll,
  merging props, polling, prefetching, once props and flash data, and the
  Inertia 3 additions, standalone HTTP requests (`useHttp`), optimistic
  updates with rollback, layout props (`useLayoutProps`), instant visits and
  SSR through the Vite plugin. The server side of each is a method on the
  `InertiaResponse` builder; [Inertia Responses](frontend-inertia-responses.md)
  lists them. Prop types (`optional`, `defer`, `merge`) take dot-notation
  paths, and a dotted partial reload narrows literal values only.
- **Names that moved.** Inertia 3 removed `Inertia::lazy()`: use
  `.optional()` for a prop skipped on the first visit. Suprnova's `.lazy()` is
  a plain callable prop that resolves on every visit that selects it (see the
  divergence callout in Inertia Responses). On the client, Axios is gone and
  the built-in XHR client with interceptors replaces it, `router.cancel()` is
  `router.cancelAll()`, the `invalid` event is `httpException` and `exception`
  is `networkError`, and the `future` configuration namespace no longer
  exists.
- **Deferred props need an empty state.** The page renders before they
  arrive, so a component that shows one carries a skeleton or a placeholder
  for the gap.
- **Tests drive the server in process.** A test calls `handle_request` and
  reads the page object with `AssertableInertia`, from a JSON visit or from
  the first-load HTML ([HTTP Tests](http-tests.md#testing-inertia-responses)).
  A page change lands with such a test.
- **Check the version.** An assistant's training data describes Inertia 1 and
  2 as often as 3. Have it read the Inertia 3 documentation and this manual
  for anything protocol-level rather than recall it.

## Next

- [Page Components](frontend-pages.md)
- [Inertia Responses](frontend-inertia-responses.md)
- [TypeScript Types](frontend-typescript-types.md)
- [Routing](routing.md)
- [Controllers](controllers.md)
