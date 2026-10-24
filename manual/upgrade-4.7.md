# Upgrading to 4.7

This chapter moves an application from Suprnova 4.6.2 to 4.7.0. Each section
matches an entry in the 4.7.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.6 to 4.7.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
Most of 4.7.0 is new features you adopt when you want them: the debug bar,
nearest-location queries, the documentation site plugin, the segmented date
picker and two graph stores. None of them changes an application that does not
use it. Three changes can need an edit: a production HydraDB store configured
over plaintext HTTP, a classic form that relied on a password field coming back
in old input, and a documentation TOC that linked outside its source folder.
Two steps are worth taking on the day you upgrade: install the debug bar, which
the scaffolds now install for new applications, and call `LoginThrottle` from
your session login, which the scaffolds now do for new applications.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.7.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.7.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.7.0 suprnova-cli
   cargo check --all-targets
   ```

2. Install the debug bar in the HTTP bootstrap if you want request diagnostics
   in development; the section below shows the two lines.
3. If you mount the documentation plugin, run `suprnova generate-types` after
   mounting it, so the frontend's `tool-schemas.ts` carries `search_docs`,
   `read_page` and `list_pages`.

## Added entries

### A debug bar.

**Before:** the framework had no per-request diagnostics beyond logging and
the development error page.

**Now:** `DebugBar::middleware()` records each request with debug on: route,
timings per middleware and handler, queries with bindings, cache, events,
mail, notifications, jobs, RenderCache, Live and the error report. A browser
response carries the toolbar under the response's CSP nonce, every response
names its record in a header, and `/_debugbar/{id}` serves the record as
JSON. With debug off the middleware passes through, keeps no record and the
route answers 404.

**Do:** a new scaffold needs nothing. In an existing application, register
it with the other global middleware before constructing the server:

```rust
use suprnova::{DebugBar, FrameworkError, SecurityHeaders, global_middleware};

pub fn register_http_stack() -> Result<(), FrameworkError> {
    global_middleware!(SecurityHeaders::from_env()?);
    global_middleware!(DebugBar::middleware()?);
    Ok(())
}
```

Set `DEBUGBAR=false` to keep it off in a development build; `DEBUGBAR_KEEP`
sets how many completed requests the in-memory store retains, 100 by default.
Production needs no change: with debug off the bar does not exist.

### Nearest-location queries.

**Before:** a model had no coordinate declaration, and a distance query was
raw SQL.

**Now:** `geo(lat = "latitude", lng = "longitude")` on a model gives its
builder `nearby(lat, lng)`, `within(radius)`, `order_by_distance()` and
`in_miles()`. The distance is computed by the haversine formula in SQL,
rendered per dialect for SQLite, Postgres, MySQL and MariaDB, and read on each
result through `distance()`. `within` prefilters with a bounding box on the two
coordinate columns. Invalid coordinates, a non-finite or negative radius, and
`within` or `order_by_distance()` without a preceding `nearby` are refused
before any SQL runs.

**Do:** nothing, unless you adopt it. Then declare the attribute, add an index
on the two coordinate columns so the bounding box bounds the rows the formula
visits, and do not name a column or field `distance` on a model that declares
`geo`: the macro refuses it because the query projects that alias. On SQLite
the framework registers the math functions on each connection when the build
lacks them; the `database-sqlite` feature now names `libsqlite3-sys` directly
at the version SQLx already carries, so the dependency tree gains no crate.

### A documentation site plugin.

**Before:** a documentation site was an application's own routes, or a static
site beside it.

**Now:** `suprnova-docs` mounts a versioned, multi-language docs site from a
`docs/` folder: a `docs.json` manifest, a `navigation.json` per version,
Markdown pages rendered through the framework's Markdown pipeline inside a
layout you supply, private pages behind a gate you supply, Markdown twins at
each page's `.md` address, `llms.txt` and `llms-full.txt` per version, an
in-page search with no service to run, and three read tools on the agents
endpoint.

**Do:** nothing, unless you adopt it. Then add `suprnova-docs` beside the
framework dependency at the same Git tag, mount it before preparing the tool
registry, and run `suprnova generate-types`. The chapter shows the layout
callback and the folder layout.

### The segmented date picker joins the Live component library.

**Before:** the Live component library had one date control, the date picker,
a `type="date"` input with a calendar.

**Now:** `suprnova.segmented-date-picker` is a second, form-associated
control: three focusable numeric segments in the order you name, `min`, `max`
and `required`, a horizontal value scroller for the focused segment, and a
keyboard edit buffer; the form sees one `YYYY-MM-DD` value and `live:model`
binds it.

**Do:** nothing. To use it, run `suprnova live:add segmented-date-picker`,
load its stylesheet and module as the Live chapter shows, and run
`suprnova live:check`.

### Neo4j and HydraDB graph stores.

**Before:** the recommender's graph store ran on Memgraph only.

**Now:** `RECOMMEND_GRAPH_STORE=neo4j` with the `graph-neo4j` feature runs it
on Neo4j 5 over Bolt, and `RECOMMEND_GRAPH_STORE=hydradb` with the
`graph-hydradb` feature runs it on HydraDB over its HTTPS JSON query endpoint.
Both read the same `RECOMMEND_GRAPH_URL`, `RECOMMEND_GRAPH_USERNAME` and
`RECOMMEND_GRAPH_PASSWORD` as Memgraph. Boot refuses an unknown store and
names the feature a selected store needs.

**Do:** nothing. Both features are off by default, like `graph-memgraph`. To
adopt one, enable its feature and set the store and connection variables; the
Recommendations chapter gives the HydraDB endpoint shape and the rebuild
command.

## Changed entries

### Classic validation redirects no longer flash secrets.

**Before:** a classic HTML form request that failed validation flashed every
field into the session's old input, passwords included.

**Now:** `password`, `password_confirmation` and `current_password` are
excluded at every nesting level, and a request excludes more names through
`FormRequest::dont_flash`.

**Do:** nothing, unless a form of yours read a password back from old input;
it will be empty, as it is in Laravel.

### HydraDB over plaintext HTTP is for development and testing only.

**Before:** `RECOMMEND_GRAPH_URL=http://127.0.0.1:...` was accepted for the
HydraDB store in every environment.

**Now:** it is accepted when `APP_ENV` is local, development or testing. In
production, which an unset `APP_ENV` means, boot refuses it and names
`https://`. The loopback client bypasses environment proxies.

**Do:** if a production application reaches HydraDB over plaintext loopback,
put TLS in front of it and use an `https://` URL before upgrading.

### The documentation builder stays inside its source root.

**Before:** a TOC link could name an absolute path, climb with `..`, or follow
a symlink out of the source folder, and the file's content was published.

**Now:** the build fails with an error that names the TOC line.

**Do:** nothing, unless your TOC linked outside the folder on purpose; move
the file inside it.

### Documentation plugin assets inherit page privacy.

**Before:** an asset carried only its set's or version's privacy.

**Now:** an asset referenced only by private pages needs the gate's
admission; an asset any public page references stays public.

**Do:** nothing. If a private page shares an asset with a public page, the
asset is public by design; give the private page its own copy to hide it.

### Process tasks report progress and may be watched for hangs.

**Before:** a child process ran until it exited or hit the deadline you set,
and the driver knew nothing about its progress.

**Now:** the driver samples each child's progress (CPU time on Linux and the
bytes it writes) and reports the last progress instant and the idle
duration. `hang_after(duration)` kills a child whose progress stopped for
that long, with `ProcessError::Hung` naming the idle time. No deadline, hang
window, concurrency ceiling or output ceiling applies unless you set it.

**Do:** nothing. To detect hung tasks, call `.hang_after(...)` with a window
measured for your workload, as [Processes](processes.md) shows; a long task
that keeps working is never killed by it.

## Security entries

### SQS overflow pointers are checked before use.

**Before:** a received message beginning with `@` named any cache or disk key,
which was read whole and deleted on acknowledgement.

**Now:** the pointer must carry the current queue's prefix and a canonical
UUID; anything else is refused before any read or delete.

**Do:** nothing, unless something other than the framework writes overflow
pointers to your queue; those pointers must carry the queue prefix and a UUID.

### Payment adapters refuse malformed signature timestamps and provider IDs.

**Before:** the Paddle verifier handed the header timestamp straight to the
SDK, and provider IDs went into request paths as given.

**Now:** an out-of-range or out-of-window timestamp is a signature error
before any SDK arithmetic, and a Stripe or Paddle ID that is not one path
segment is a validation error before any request.

**Do:** nothing. An ID that carried a slash or a query never reached the
provider correctly before.

### A login attempt budget for session sign-in.

**Before:** nothing in the framework or the scaffolds limited repeated wrong
passwords.

**Now:** `LoginThrottle` counts failures per lower-cased email and client IP,
five per sixty seconds, reports the seconds to wait, clears on success and
dispatches `Lockout`. New scaffolds call it before `Auth::attempt`.
`Auth::attempt` is unchanged.

**Do:** in an existing application's login handler, replace the
`Auth::attempt` call with
`LoginThrottle::for_request(&email, &request).attempt(&credentials, remember).await?`.
When a `FormRequest` consumes the request first, install
`LoginThrottle::request_context()` on the login route and use
`LoginThrottle::for_current_request(&form.email)?` instead, as
[Authentication](authentication.md) shows. Without that call nothing changes.

## Fixed entries with no upgrade step

- **`suprnova live:check` no longer reports a missing view for a Live
  topic.** A whole-application check that failed only on a
  `suprnova.topic.<hash>` component passes now. Nothing to change.

## Next

- [Debug Bar](debugbar.md) documents the panels, the gate and the record
  endpoint.
- [Eloquent](eloquent.md) documents nearest-location queries with the other
  query builder methods.
- [Documentation sites](docs-site.md) documents the plugin's folder layout,
  manifest, private pages and tools.
- [Live](live.md) documents the component library, the segmented date picker
  included.
- [Recommendations](recommendations.md) documents the graph stores.
- [Authentication](authentication.md) documents the login attempt budget.
- [Processes](processes.md) documents progress and `hang_after`.
- [Validation](validation.md) documents old input and `dont_flash`.
- [Upgrading to 4.6](upgrade-4.6.md) covers the preceding release.
