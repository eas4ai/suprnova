# RenderCache

RenderCache stores a proven-safe copy of a GET or HEAD route's response and
serves the next matching request from it without running your handler at
all. You opt routes and groups in explicitly; everything else keeps working
exactly as it does today. A route you never opt in is untouched. A route you
do opt in still renders and serves correctly even when nothing about that
particular request turns out to be safe to cache - it just never gets
stored, and you can find out why.

This chapter covers enabling the cache, opting routes and groups in,
declaring variance, reading the response headers it adds, the reasons a
render is declined, operational control, and how it differs from
`suprnova::Cache`.

## The chapters

This is the first of five. Read them in order the first time; after that,
each answers one question on its own.

| Chapter | Answers |
|---|---|
| RenderCache (this one) | How do I turn it on and opt a route in? |
| [Representations](render-cache-representations.md) | What is actually stored, and under what key? |
| [Generations](render-cache-generations.md) | When does a stored copy stop being current? |
| [Deployment](render-cache-deployment.md) | How do several nodes share one cache? |
| [Operations](render-cache-operations.md) | How do I inspect it, test it, measure it, and switch it off? |

## Enabling the cache

Two environment variables matter to start:

- `RENDER_CACHE_ENABLED` - `true` unless set to `false` or `0`. With it
  disabled, every request bypasses RenderCache entirely; nothing is looked
  up and nothing is stored.
- `RENDER_CACHE_L1_DIR` - unset by default, which means no on-disk tier. Set
  it to a directory the process can create and write to, and stored
  representations survive a process restart in a file-backed second tier.

A handful of other variables tune the defaults: `RENDER_CACHE_L0_ENTRIES`
(4,096) and `RENDER_CACHE_L0_BYTES` (128 MiB) bound the in-process tier;
`RENDER_CACHE_L1_BYTES` (1 GiB) bounds the file tier; `RENDER_CACHE_FAILURE`
(`open` by default, or `closed`) decides whether a store or database problem
serves the route uncached or refuses the request; `APP_BUILD_ID` namespaces
every cached entry to the build that produced it. Set it explicitly to
something that changes every deploy: its default is a compiled-in crate
version, which does not. See
[RenderCache Deployment](render-cache-deployment.md).

`RENDER_CACHE_PROFILE` (`embedded` by default, or `database` or `redis`)
chooses whether the second tier and the rebuild coordinator are in this
process or shared with every other node. A shared profile also needs a
migration your application lists. Both are the
[Deployment](render-cache-deployment.md) chapter's subject, together with
the full variable table.

## Opting a route or a group in

Nothing is cached until you say so. `Router::try_render_cache` opts one
already-registered route pattern in; `Router::try_render_cache_group` opts
every route under a path prefix in. Both take a policy built with
`RenderCachePolicy::builder`:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` sets
how long a representation is fresh, and then two windows measured from that
fresh edge: how far past it the stored copy may still be served while a
background rebuild runs, and how far past it the stored copy may be served
if a foreground rebuild fails outright. The two windows are not stacked; see
[RenderCache Representations](render-cache-representations.md).

`RepresentationClass` runs from widest to narrowest sharing: `PublicShared`
(one representation for everyone who matches the declared variance),
`PublicShellStitched` (a Live document whose shared shell is stored once and
whose islands are re-mounted for whoever is asking; see
[Representations](render-cache-representations.md)),
`PrivateCached` (one representation per signed-in visitor or tenant), and
`Uncacheable`.

A route pattern must already be registered before you opt it in, and you
must finish opting routes and groups in **before** calling
`RenderCache::install` (below) - the install step reads whatever has been
registered by that point.

A route-level policy can also be a narrowing patch of its enclosing group,
using `PolicyPatch` instead of a full `RenderCachePolicy`: it inherits
everything the group declared and may only make it narrower (a shorter
freshness window, a stricter class), never wider. Pulling one route out of a
cached group entirely is a `PolicyPatch` that sets the class to
`Uncacheable`.

Finish wiring RenderCache in with one line, after every middleware
registration that establishes request-scoped locale, session, or identity
(RenderCache reads them to build its lookup key, so it needs to run after
whatever sets them up):

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## Declaring variance

By default a cached representation varies only by route pattern, path
parameters, and the application build. Anything else your handler's output
actually depends on needs to be declared, with two mechanisms:

- **Query parameters.** `.query(QueryPolicy::declared(["page", "sort"]))`
  names the query parameters that distinguish representations; any other
  query parameter present on a request bypasses the cache for that request
  rather than being silently ignored.
- **Variance dimensions**, added one at a time with `.vary(dimension)`:
  - `VarianceDimension::Locale` partitions by the negotiated locale.
  - `VarianceDimension::Host` partitions by the request's host, where your
    deployment makes more than one host meaningful.
  - `VarianceDimension::Tenant` partitions by the current tenant as opaque
    key material; a route whose handler ever reads the tenant must declare
    it.
  - `VarianceDimension::Principal` partitions by the signed-in visitor as
    opaque key material, bound to a permission version (see "Epoch,
    permissions, and inspection" below); a `PrivateCached` route must
    declare `Principal` or `Tenant` (or both) or it fails to build at all.
- **`Media` and `Encoding`**, declared together with their own closed set:
  `.vary_media(NegotiatedPolicy::declared(["text/html", "application/json"],
  "text/html")?)` and `.vary_encoding(NegotiatedPolicy::declared(["identity",
  "gzip"], "identity")?)`. The bare `.vary(VarianceDimension::Media)` (or
  `::Encoding`) is rejected at `build`/`apply`: unlike every other
  dimension, these two negotiate against a set only the route can name, so
  there is nothing to key by without it.

  Negotiation reads the request's `Accept` (for `Media`) or
  `Accept-Encoding` (for `Encoding`) header, matches it against the
  declared set, and adds the matching request header to `Vary`. It is
  `q`-weighted: the declared member with the highest quality wins, and
  equal quality keeps the header's own left-to-right order, so the
  candidate listed first wins a tie. A wildcard (`*/*`, `type/*`, a bare
  `*`) is compared as a literal token, not expanded against the set, so it
  practically never matches a real declared value. An absent header, a
  value naming nothing in the declared set, or a header this cannot make
  sense of - a `q=0`, an out-of-range or unparsable quality, garbage syntax
  - resolves to the declared default rather than creating a variant or
  failing the request. Two different negotiated values are two different
  keys; the same negotiated value, however it was spelled or weighted on
  the wire, is always the one stored representation for it.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion`, and
a custom `VarianceDimension::Application(name)` exist on the type but have
no resolver in this release: a route that declares one bypasses the cache
on every request, silently, rather than failing to build. Do not declare
them yet.

## Reading the response headers

A served hit carries `ETag` (a strong validator your client can send back as
`If-None-Match` for a `304`), `Cache-Control`, `Vary`, and `Age` (whole
seconds since the representation was published, and the quickest local sign
that a response came out of the store rather than out of your handler). A
response served past its fresh interval additionally carries
`Warning: 110 - "Response is Stale"`. Each of the five is defined, with the
values the dogfood routes are asserted to send, in
[RenderCache Representations](render-cache-representations.md).

## Why a render is never stored

Being opted in is not a guarantee. Two independent checks run after every
render, and either can decline storage without failing the request - the
response you get back is identical either way, it just never becomes a
cache entry:

**Eligibility** declines outright for a response that is not a plain `200`
to a `GET` or `HEAD`, that streams its body, that sets a cookie, or that
carries a hop-by-hop or tracing header. These are almost always accidental
(a redirect, an error page, a response that happens to touch
`Set-Cookie`) rather than something you need to design around.

**Classification** declines based on what your handler actually did while
it ran, in terms you will recognize:

- **You read a session value.** Any read of the current session (through
  `session()`, `session_mut`, or a session cookie) forces the render to
  `Uncacheable`, permanently, no matter what variance the route declares.
  The one thing this does *not* cover is the signed-in visitor's own
  identity. `Auth::id()` reads it out of the session when nothing earlier in
  the request resolved it, and that read is classified as an identity read,
  not a session read - so an ordinary cookie-backed login is exactly what a
  `PrivateCached` route declaring `Principal` variance is for, and reaching
  for the visitor's id does not quietly make the page uncacheable. Every
  other value in the session still does. Two consequences worth knowing: an
  anonymous request to such a route caches under the `Anonymous` key,
  because the render resolved no identity, observed no principal material,
  and the key says so - a signed-in visitor derives a `Private` key that
  never reaches that entry; and a named guard's own identifier is principal
  material in exactly the same way as the default guard's.
- **You read an identity, on a route that does not declare `Principal`.**
  Reading the signed-in user narrows the class to `PrivateCached`; if the
  route's declared variance does not include `Principal`, there is no way
  to key the entry per visitor, so it is declined rather than shared.
- **You translated (or your view engine did) without declaring `Locale`.**
  Any read of the negotiated locale needs a declared `Locale` dimension, or
  the render is declined. Every Inertia page's document shell reads the
  locale to set `<html lang>`, whether or not the page's own data has
  anything to do with language - so an Inertia route needs `Locale`
  declared to ever cache at all, even one with no translated content of its
  own.
- **You checked authorization.** A decision is judged by what its own
  evaluation read. A gate whose body reads only the tenant - through
  `suprnova::live::current_tenant()`, say - classifies under `Tenant` alone
  and caches on a route keyed by `Tenant`. A gate that reads a per-user
  fact, or that reads nothing RenderCache can see, still needs `Principal`
  declared: a body that decided from its `user` argument through no
  instrumented accessor is indistinguishable from one that decided from a
  constant, and the safe reading of that is the conservative one.
- **A model behind the page carries a global scope that reads per-request
  state.** Declare what the scope depends on. A `GlobalScope` returning
  `ScopeDependency::Constant` records nothing and costs no cache hits. The
  default, `ScopeDependency::PerRequest`, requires the scope's `apply` to
  read that state through an instrumented accessor -
  `suprnova::live::current_tenant()`, `Auth::id()`, `Lang::locale()`. A
  per-request scope whose evaluation reads none of them narrows the render
  to `Uncacheable` and names itself in the decline, so an invisible tenant
  filter costs you the cache rather than costing your visitors each other's
  rows.
- **You read a secret configuration value, or an undeclared request
  context.** Both force `Uncacheable`. A response's dependence on an
  ordinary request header, or on `Config::get`, is invisible to RenderCache
  entirely - it cannot decline what it cannot see, so declaring the
  matching variance is on you.
- **You ran raw SQL through `DB::select`, `DB::select_one`, `DB::scalar`, or
  `DB::select_on`.** The framework cannot name the tables a raw statement
  read, so the render is never stored; it is still served. Reads through
  `DB::table(..)` know their table and are cached normally, and so is
  `Auth::user()`, which resolves through that path.
  The framework's own RBAC role and permission checks name the five tables
  they read - `roles`, `permissions`, `role_permissions`, `model_roles`, and
  `model_permissions` - so a cached route that evaluates one is observed
  precisely and cached normally.
- **The write was made by a queue worker, a scheduled task, or a console
  command.** Nothing special is needed any more. Every process whose
  configuration enables RenderCache and whose database holds the RenderCache
  migration advances generations, so such a write invalidates exactly what
  the same write invalidates in the server, and
  `RenderCache::bump_permission_version()` works from any of them. A process
  with `RENDER_CACHE_ENABLED=false`, or one whose database does not hold the
  migration, advances nothing and issues no RenderCache SQL at all.

On PostgreSQL the render runs in a `REPEATABLE READ` transaction so that what
it read and the generations it recorded agree; a cached route's handler that
updates a row another transaction changed after the render began sees a
serialization failure. Design cached routes as read paths. A handler that
does write inside the render transaction still advances generations, but it
competes with concurrent writers for the same rows and can see the
serialization failure above.

A write made outside any transaction (`model.save()` on its own) commits
first and advances its generations in an immediately following transaction,
so the moment between the two is "new data, old generation": one extra
rebuild, never stale content.

None of this needs special tooling to see happen in practice: the hidden
`render-cache:inspect` command (below) shows whether a route's entry
exists at all, or you can just try two requests in a row and check whether
the second one carries an `Age` header.

## A route that caches

A public listing page with no per-visitor content:

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

registered and opted in:

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` never touches the session, the signed-in visitor, or the locale, so
the first request renders and publishes; every request for the next five
minutes is served from that stored copy with an `Age` header, a `304` for a
client that already has it, and `Cache-Control: public, max-age=300,
s-maxage=300` for any CDN in front of it.

## A route that is declined

The same shape of page, but the handler reads the session to show a flash
message:

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

opted in exactly the same way as above. Every request still renders and
serves the correct page - flash message included - but nothing is ever
stored: the session read narrows the class to `Uncacheable` before
RenderCache even reaches the eligibility check, so a second request for the
same URL renders again from scratch rather than coming back with an `Age`
header. The fix, if this page is meant to cache, is to stop reading the
session in the cached path (render the flash from a query parameter or a
separate small response instead) - there is no variance declaration that
makes a session read cacheable, because a session read means the response
depends on something no key could safely partition by.

## Epoch, permissions, and inspection

- **`RenderCache::bump_permission_version().await?`** - call this whenever
  an application action changes what a signed-in user is allowed to do (a
  role change, a permission grant or revocation). It advances a persisted
  generation that every principal-keyed render observes. The generation
  survives a restart, and the bump joins the transaction the role change
  runs in when there is one. Without the bump, a user whose permissions just
  changed keeps matching whatever was cached under their prior permission
  set.
- **`RenderCache::advance_epoch()`**, or the hidden
  `render-cache:epoch-advance` command - an emergency invalidation. The
  epoch is baked into the lookup key itself, so advancing it puts stored
  entries out of reach with nothing to enumerate and nothing to delete. On
  the process that runs it the effect is immediate: it drops that process's
  epoch lease and clears its in-process tier the same instant. Another node
  catches up at its next authority read, and its file-backed tier keeps its
  old files until a sweep reclaims them - the automatic one every 256th
  publication, or an explicit `RenderCache::sweep()` - which is disk hygiene
  rather than a correctness concern. Reach for this when something
  is wrong with cached content and you cannot wait for individual entries to
  expire; on more than one node, see
  [RenderCache Operations](render-cache-operations.md).
- **The hidden `render-cache:inspect <key>` command** reports one stored
  entry's metadata (never its body) by the key text your application logs
  or telemetry can surface, alongside the current epoch, so you can tell
  whether what you are looking at is still live authority or has already
  aged out from underneath. It looks the key up in the running process's
  in-process tier only, never in the shared one, so on a `database` or
  `redis` profile it reports no entry for a key this node has not served
  itself.

## RenderCache versus `suprnova::Cache`

`suprnova::Cache` is a key-value store you call explicitly: you choose the
key, you choose what to store, you choose when to invalidate it
(`Cache::put`, `Cache::get`, `Cache::remember`, `Cache::forget`). It works
for any data your code decides is worth caching, on any backend you
configure (memory or Redis).

RenderCache is not a general-purpose store, and you never call it from your
handler. It caches whole HTTP responses, the key is derived automatically
from the route and its declared variance, and invalidation is
generation-based: an ordinary database write through the ORM or query
builder advances the generations the render depended on, and the entry is
recomputed the next time it is asked for rather than deleted by hand; a
render that read raw SQL is never stored in the first place, so there is
nothing to recompute. Reach
for `suprnova::Cache` when you have a specific value you want to compute
once and reuse; reach for RenderCache when you have a whole route whose
response is expensive to render and safe to share.

### Why Suprnova diverges

Laravel has no equivalent in the framework itself. Response caching is a
package you add, it wraps the route in middleware that stores the rendered
response under a key you compose, and everything after that is yours: which
routes are safe to cache, what makes two visitors different, and when a
stored page stops being true. The framework does not know a page was cached,
so it cannot tell you when caching one was a mistake.

RenderCache is part of the framework for exactly that reason. It sees the
render happen, so it can record what the handler read, compare that against
what the route declared, and refuse to store a response whose safety it
cannot account for - silently, without changing what the visitor is served.
Opting a route in is a declaration the framework then holds you to, rather
than a promise you make to yourself. The cost is that some routes you would
like to cache are declined and you have to find out why; the benefit is that
the ones that are stored were proven safe to store, once, by the process
that rendered them.

## Next

- [RenderCache Representations](render-cache-representations.md) - what is
  actually stored, under what key, and in which layers
- [RenderCache Generations](render-cache-generations.md) - how a stored copy
  stops being current
- [Cache](cache.md) - the explicit key-value store this chapter contrasts
  with
- [Live](live.md) - the documents a stitched representation is cut from
