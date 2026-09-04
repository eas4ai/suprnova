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
serves the route uncached or refuses the request; `APP_BUILD_ID` (your
crate's own version by default) namespaces every cached entry to the build
that produced it, so a deploy never serves an old build's bytes.

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
            .freshness(
                FreshnessPolicy::new(300_000, 60_000, 300_000)
                    .map_err(FrameworkError::from_external)?,
            )
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()
            .map_err(FrameworkError::from_external)?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` sets
how long a representation is fresh, how much longer it may still be served
while a background rebuild runs, and how much longer still it may be served
if that rebuild fails outright. `RepresentationClass` runs from widest to
narrowest sharing: `PublicShared` (one representation for everyone who
matches the declared variance), `PublicShellStitched` (reserved for a future
composed-shell representation, not usable yet), `PrivateCached` (one
representation per signed-in visitor or tenant), and `Uncacheable`.

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

let router = add_render_cache(router)?;
let router = RenderCache::install(router, RenderCacheConfig::from_env()).await?;
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
  - `VarianceDimension::Media` partitions by the negotiated media type.
  - `VarianceDimension::Host` partitions by the request's host, where your
    deployment makes more than one host meaningful.
  - `VarianceDimension::Tenant` partitions by the current tenant as opaque
    key material; a route whose handler ever reads the tenant must declare
    it.
  - `VarianceDimension::Principal` partitions by the signed-in visitor as
    opaque key material, bound to a permission version (see "Epoch,
    permissions, and inspection" below); a `PrivateCached` route must
    declare `Principal` or `Tenant` (or both) or it fails to build at all.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion`, and
a custom `VarianceDimension::Application(name)` exist on the type but have
no resolver in this release: a route that declares one bypasses the cache
on every request, silently, rather than failing to build. Do not declare
them yet.

## Reading the response headers

A served hit carries `ETag` (a strong validator your client can send back as
`If-None-Match` for a `304`), `Cache-Control` (`private` unless the class is
`PublicShared` and you set a `SharedCachePolicy::SMaxAge`, in which case it
also carries `public` and `s-maxage`), `Vary` (from whichever declared
dimensions imply one - `Locale` implies `Accept-Language`, `Media` implies
`Accept`), and `Age` (whole seconds since the representation was published).
A stale-servable response additionally carries `Warning: 110 - "Response is
Stale"`.

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
  This also fires when an anonymous visitor's identity resolves through the
  session fallback - a common surprise, since the visitor is genuinely
  anonymous and the resulting key is correctly `Anonymous`, but the read
  itself is still a session read.
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
- **You checked authorization.** `Gate` always treats a decision as
  per-visitor, so it needs `Principal` declared even on a route keyed only
  by `Tenant`, until the gate's own check is provably per-tenant.
  RenderCache cannot tell the difference on its own.
- **A model behind the page carries a tenant-scoped global scope.** A
  global scope that reads the current tenant from its own request-local
  state to filter a query - the pattern Suprnova's own `GlobalScope`
  documentation shows - changes what the query returns without RenderCache
  ever seeing that read. Declare `Tenant` variance on any route backed by
  such a model; nothing here can catch the omission for you.
- **You read a secret configuration value, or an undeclared request
  context.** Both force `Uncacheable`. A response's dependence on an
  ordinary request header, or on `Config::get`, is invisible to RenderCache
  entirely - it cannot decline what it cannot see, so declaring the
  matching variance is on you.

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
use suprnova::{FrameworkError, get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000).map_err(FrameworkError::from_external)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()
        .map_err(FrameworkError::from_external)?,
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

- **`RenderCache::bump_permission_version()`** - call this whenever an
  application action changes what a signed-in user is allowed to do (a role
  change, a permission grant or revocation). Without it, a user whose
  permissions just changed keeps matching whatever was cached under their
  prior permission set.
- **`RenderCache::advance_epoch()`**, or the hidden
  `render-cache:epoch-advance` command - an emergency invalidation. Every
  currently stored entry becomes unreachable by ordinary lookup at its very
  next request, immediately, because the epoch is baked into the lookup key
  itself. The in-process tier is also cleared outright the same instant; a
  file-backed tier keeps its old files on disk until the periodic or manual
  sweep reclaims them, which is disk hygiene rather than a correctness
  concern. Reach for this when something is wrong with cached content and
  you cannot wait for individual entries to expire.
- **The hidden `render-cache:inspect <key>` command** reports one stored
  entry's metadata (never its body) by the key text your application logs
  or telemetry can surface, alongside the current epoch, so you can tell
  whether what you are looking at is still live authority or has already
  aged out from underneath.

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
recomputed the next time it is asked for rather than deleted by hand. Reach
for `suprnova::Cache` when you have a specific value you want to compute
once and reuse; reach for RenderCache when you have a whole route whose
response is expensive to render and safe to share.
