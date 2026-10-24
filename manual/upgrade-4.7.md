# Upgrading to 4.7

This chapter moves an application from Suprnova 4.6.2 to 4.7.0. Each section
matches an entry in the 4.7.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.6 to 4.7.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
All of 4.7.0 is new features you adopt when you want them: the debug bar,
nearest-location queries, the documentation site plugin, the segmented date
picker and two graph stores. None of them changes an application that does not
use it. Two steps are worth taking on the day you upgrade: install the debug
bar, which the scaffolds now install for new applications, and regenerate the
frontend types if you mount the documentation plugin, because it adds three
tools to the agents endpoint.

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
- [Upgrading to 4.6](upgrade-4.6.md) covers the preceding release.
