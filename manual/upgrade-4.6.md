# Upgrading to 4.6

This chapter moves an application from Suprnova 4.5.1 to 4.6.0. Each section
matches an entry in the 4.6.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.5 to 4.6.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
Most of 4.6.0 is new features you adopt when you want them: static
publication of public Inertia pages, the page budget audit, Inertia pages in
RenderCache, Live bindings for Inertia pages and event sourcing. None of them
changes an application that does not use it. Three changes can need an edit:
a cacheable Inertia route whose shared props come from an opaque hook, code
that relied on the old JPEG quality, and a dependency list that wants to drop
the new default CloudFront feature.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.6.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.6.1" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.6.1 suprnova-cli
   cargo check --all-targets
   ```

2. Boot the application once in development. A cacheable Inertia route with an
   opaque share hook now fails boot with the route name; the section below says
   how to declare its shared props.
3. Run `suprnova generate-types`, so the frontend's generated files match what
   the server publishes.

## Changed entries

### The default lossy encode quality is 85, not 70.

**Before:** JPEG and lossy WebP encoded without an explicit `quality` call
used 70.

**Now:** they use 85, which `DEFAULT_IMAGE_QUALITY` names. Images are larger
and sharper. The built-in driver also honours the setting for JPEG now; see
the fixes list.

**Do:** nothing, unless you depended on the smaller files. Then call
`quality(70)` on the pipelines that need it.

### Inertia protocol middleware runs before RenderCache

**Before:** the order in which the application registered the Inertia
protocol middleware and RenderCache decided which ran first.

**Now:** the protocol middleware always runs first, so an asset version
mismatch is answered before a stored page could be replayed. A cacheable
Inertia route with an opaque share hook or provider fails boot with the route
name, because RenderCache cannot tell which shared props it reads.

**Do:** if boot names a route, declare its shared props by name as
[Render Cache](render-cache.md) shows, instead of the opaque hook.
Middleware you box through `into_boxed` keeps its type identity, so the
ordering and boot checks recognise it.

### Static files get a cache policy by file name.

**Before:** `StaticFiles` applied `cache_control(...)` to every file and sent
no `Cache-Control` when it was unset. A static file response could set the
session and XSRF cookies.

**Now:** a fingerprinted file (Vite's `name-<hash>` output, confirmed by the
build manifest when one is configured) gets
`public, max-age=31536000, immutable`, and every other file gets
`public, max-age=3600` unless `cache_control(...)` sets another value. A
response no longer sets the session or XSRF cookie when the request left the
session untouched.

**Do:** nothing for a Vite build. If you serve files whose names end in a
hyphen and eight characters but whose content changes in place, rename them or
call `fingerprint(pattern)` with a pattern that excludes them. If you set
`cache_control` expecting it to cover fingerprinted files too, note that those
now always get the one-year policy.

### `cdn-cloudfront` is a default feature.

**Before:** the framework had no CDN driver.

**Now:** the CloudFront driver for static publication is on by default. It
reuses the AWS signing stack the S3 disk and the SQS queue driver already
carry and adds `quick-xml`.

**Do:** nothing. To drop it, set `default-features = false` on the
`suprnova` dependency and list the default features you keep.

### `LiveStreams::refresh` follows the shared topic rules

**Before:** a stream refresh published at once, outside the cursor, rate and
socket rules that Live topics use.

**Now:** it follows the same cursor, fanout, scope, sender exclusion and rate
rules as a topic change, and `LiveStreamErrorKind::RuntimeUnavailable` also
covers a configured publisher that is unavailable.

**Do:** nothing. A refresh inside a request or a job is now published after
the scope ends and its transaction commits, so a test that read the
publication before the request returned reads it after.

## Fixed entries that change behaviour

### A savepoint rollback discards the callbacks of the work it rolls back.

**Before:** a nested transaction that completed and was then rolled back to an
enclosing savepoint could still run its after-commit callbacks when the outer
transaction committed, for rows that no longer existed. A panicking callback
stopped the callbacks after it.

**Now:** each savepoint and the callbacks registered after it stay together:
rolling back to the savepoint runs the rollback callbacks of the discarded
work and drops its after-commit callbacks. A panicking callback is logged and
the rest still run.

**Do:** nothing, unless code relied on the old behaviour. Search for
`after_commit` inside nested `DB::transaction` closures that roll back to a
savepoint and check that the callback only describes committed work.

### Tool schemas publish integer fields with their bounds and no JSON Schema format.

**Before:** an integer field's schema carried the Rust type's format, such as
`"format": "uint64"`, which clients do not know; Claude Code logged
`unknown format "uint64" ignored` for each one.

**Now:** the schema says `"type": "integer"` with the field's `minimum` and
`maximum` and no `format`. The WebMCP browser schemas follow the same rule.

**Do:** run `suprnova generate-types` once after the bump, so the frontend's
`tool-schemas.ts` matches what the server publishes.

## Fixed entries with no upgrade step

- **The MariaDB vector driver reads a quoted index distance.** On MariaDB
  12.3.3 every `similar` call failed with a distance mismatch against a table
  the driver had created itself. It now works on 12.3.3 and on earlier
  releases. Nothing to change.
- **The built-in driver honours the JPEG quality setting.** The OxideAV
  driver wrote every JPEG at 75 whatever the pipeline asked. It now writes the
  quality you set, 85 by default. Nothing to change.
- **Two tools with one name in different crates now fail the build at link
  time.** An application that built and booted before has no such duplicate,
  because the boot check refused it; the failure moves to the link step, where
  the duplicate symbol names the tool. A crate that forbids unsafe code still
  builds. Nothing to change.
- **A fresh scaffold passes its own page budget test and `cargo fmt --check`.**
  In a React, Vue or Svelte application scaffolded from 4.6.0, make
  `tests/page_budget.rs` audit the router `cmd/main.rs` serves: register
  `live::registry()` and the notes gate rule, then audit
  `live::routes_with_render_cache(agents::serve(routes::register())?)`
  prepared with `suprnova::live::testing::prepare_live_router_for_test`. Run
  `cargo fmt` once. Nothing else changes. (4.6.1)
- **A Live action's framework frames fit their stack budget again in a debug
  build.** A debug build of a 4.6.0 application could overflow Tokio's 2 MiB
  stack in a deep Live action. Nothing to change once you update. (4.6.1)
- **The page budget audit's URL mode counts the bytes that cross the wire.**
  A URL audit of a page that serves gzip now reports smaller transfer sizes,
  matching a HAR capture of the same visit. Recheck any budget you raised to
  get a URL audit to pass. (after 4.6.1)
- **4.6.2 adds to the search and error surfaces; nothing to change.** The
  `Embedder` trait gains `embed_documents_metered`, `embed_query_metered` and
  `resolve_profile`, and `SearchEngine` gains `release_creation_lease`, all
  with defaults, so a provider or driver you wrote keeps building. Use
  `HttpEmbedder::with_transport` to send embedding requests through your own
  HTTP client, `LazyEmbedder` to register a local model that loads on first
  use, `generations::reclaim_generation` or `search:reclaim` to resume a
  generation an interrupted build left behind, `CloudflareDriver` to publish
  public pages to an R2 bucket behind a Cloudflare custom domain, and
  `HttpResponse::error_view()` on a 5xx page you build yourself so the
  production error view sends it as built. (4.6.2)
- **`cdn-cloudflare` is a default feature.** The Cloudflare driver is on by
  default beside CloudFront and adds no dependency; Cloudflare with R2 is the
  recommended CDN. Nothing to change; an application built with
  `default-features = false` adds `cdn-cloudflare` when it constructs
  `CloudflareDriver`. (4.6.2)

## Next

- [Static Publication](static-publication.md) documents public pages and the
  CDN driver.
- [Page Budget](page-budget.md) documents the audit and its rules.
- [Event Sourcing](event-sourcing.md) documents aggregates, projectors and
  replay.
- [Upgrading to 4.5](upgrade-4.5.md) covers the preceding release.
