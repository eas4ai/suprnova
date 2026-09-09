# RenderCache Deployment

One process caching for itself needs nothing but memory. Several processes
behind a load balancer need to agree on what is stored, on who is allowed to
rebuild an entry, and on when something stopped being true - and they need
to agree without any of them being able to convince the others that stale
content is current. RenderCache answers that with **profiles**: a profile
names which providers a process builds, and nothing else changes. Route
declarations, policies, keys, the collector, the middleware flow, and
stitching are identical at every profile, and no application-facing type
differs between them.

This chapter is how you choose and wire one. The three profiles and what
each provides, the environment variables the framework config actually
reads, the migration your application has to list before a shared profile
will boot, where the install goes in your bootstrap, and what the tiers do
and do not promise. The dogfood application in this repository runs the
embedded profile by default and is booted on the Database profile by
`the_database_profile_serves_a_hit_through_the_sql_stores` in
`app/tests/live_render_cache.rs`.

## Three profiles

| Profile | L1 entries | Rebuild leadership | Live instance records |
|---|---|---|---|
| `embedded` (default) | one file per key, or none | in process | in process |
| `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances`, `suprnova_live_promotions` |
| `redis` | one Redis hash per key | one Redis hash per key, plus a token counter | one Redis hash per record |

The thing that does **not** move between them is generation truth. The
database-backed generation ledger is the authority at every profile:
whichever tier handed over the bytes, currency is proved against the
database - by rereading it on the hit under `CoherenceMode::Authority`, or
by a validation lease granted from an earlier read of it under
`CoherenceMode::Lease`. That is what keeps an accelerator an accelerator:
Redis can lose everything it holds without anything stale being proven
current, because nothing Redis holds proves currency in the first place.

Choose by what you actually need to share:

- **`embedded`** for a single process, and for several processes that are
  happy to each keep their own copy. Set `RENDER_CACHE_L1_DIR` and each
  process gains a file tier that survives its own restart.
- **`database`** when several nodes should share stored entries and elect
  one rebuild leader per key, and you would rather not add another moving
  part to the deployment.
- **`redis`** when the shared tier's latency matters more than its
  durability, with the database still holding generation truth underneath.

## The environment variables

`RenderCacheConfig::from_env` reads these, in
`framework/src/render_cache/config.rs`:

| Variable | Default | Meaning |
|---|---|---|
| `RENDER_CACHE_ENABLED` | `true` | anything but `false` or `0`; `false` makes `RenderCache::install` a no-op |
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database`, or `redis`; sets the two rows below |
| `RENDER_CACHE_L1` | the profile's | `disabled`, `file`, `database`, or `redis` |
| `RENDER_CACHE_COORDINATOR` | the profile's | `local`, `database`, or `redis` |
| `RENDER_CACHE_L0_ENTRIES` | 4,096 | in-process entry ceiling |
| `RENDER_CACHE_L0_BYTES` | 128 MiB | in-process byte ceiling |
| `RENDER_CACHE_L1_DIR` | unset | the file tier's directory; under `embedded`, setting it is what turns L1 on |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | the whole directory for the file tier, one entry for the database and Redis tiers |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, then `redis://127.0.0.1:6379` | where both Redis cache tiers connect |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | the key namespace both Redis cache tiers write under |
| `RENDER_CACHE_LEASE_MS` | 30,000 | rebuild lease lifetime |
| `RENDER_CACHE_MAX_WAITERS` | 128 | in-process waiter ceiling |
| `RENDER_CACHE_FAILURE` | `open` | `open` serves the route uncached on a provider failure, `closed` answers `503` |
| `APP_BUILD_ID` | a compiled-in crate version (see below) | namespaces every entry to the build that produced it |

The profile is a shorthand, not a lock. `RENDER_CACHE_L1` and
`RENDER_CACHE_COORDINATOR` each override their own half, so a deployment
that wants its entries in the database but its rebuild leases in process
says exactly that rather than picking the nearest whole profile.

A variable with a closed set of accepted values that is set to something
outside it fails the boot with a message naming the variable. The rejected
value is never repeated in that message, because an environment value can
carry a secret.

**Set `APP_BUILD_ID` explicitly, once per deploy.** It is mixed into every
lookup key, so changing it is what stops a new build from serving entries
the previous one published. Its default is not what the name suggests:
`RenderCacheConfig::from_env` falls back to `env!("CARGO_PKG_VERSION")`,
which expands at compile time inside the `suprnova` crate, so the default is
the **framework** crate's version. It equals your application's version only
because both take `version.workspace = true` from the same workspace, and
either way it moves only when someone bumps a version number. A deploy that
changes a template, a translation, or a handler without a version bump keeps
the same build id and can serve entries the previous build published. Set it
to something that changes every time you ship - a commit id or a release
identifier:

```bash
APP_BUILD_ID=$(git rev-parse --short HEAD)
```

Whichever value you set, the production binary that reads it is expected
to be built in Suprnova's [production build shape](deployment.md#production-build-shape) -
default features off, `testing` reserved for `cargo test` alone.

Live's own instance ledger is configured separately, because it is Live's
authority rather than the cache's storage: `LIVE_LEDGER_DRIVER` (`memory`,
`database`, or `redis`), `LIVE_REDIS_URL`, and `LIVE_REDIS_PREFIX`. A
deployment may run the cache on one tier and the ledger on another.

## The migration your application must list

RenderCache's schema is framework-owned and application-applied. Your
`Migrator` lists it, so `suprnova migrate` provisions the tables alongside
your own:

```rust
Box::new(suprnova::render_cache::migration::Migration),
Box::new(suprnova::render_cache::migration::TierMigration),
```

That is `app/src/migrations/mod.rs` verbatim, and the two are not
interchangeable:

- **`Migration`** creates the three `suprnova_render_*` tables that hold
  durable generation truth: current generations, an append-only change log,
  and the authority epoch. Every profile needs it, including `embedded`,
  because generation truth never moves into a cache tier.
- **`TierMigration`** creates the four tables the database L1 store and the
  database rebuild coordinator read. Only a profile that reaches them needs
  it - but `RenderCache::install` refuses to boot the Database profile
  without them, so listing it is what makes `RENDER_CACHE_PROFILE=database`
  a configuration choice your application can actually make.

An application that sets `RENDER_CACHE_ENABLED=false` need not carry either:
the install returns the router untouched, probes nothing, assembles no
runtime, registers no middleware, and leaves the write side uninstrumented,
so nothing pays for a cache that is off.

## Installing it

`RenderCache::install` is asynchronous, because it probes for the tables and
pings every distinct Redis endpoint the configuration would use before it
assembles anything. `Application::try_routes_async` is the hook that hosts
it. This is `app/src/live/mod.rs`, and the split into two functions is worth
copying:

```rust
/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
```

`routes` is the synchronous inner half: it registers the reserved Live
routes, the document routes, and every cache policy, and installs no
middleware. `cmd/main.rs` reaches `routes_with_render_cache` through
`Application::try_routes_async`, and the browser scenario's server in
`app/examples/live_dogfood_host.rs` awaits it directly.

The configuration seam underneath it is not decoration. A test that needs a
different profile or a clock it can move has no other way in, and it matters
that it installs *the same* routes, policies, and middleware ordering the
server does, differing only in the configuration it passed. Both dogfood
boots go through it: `the_database_profile_serves_a_hit_through_the_sql_stores`
passes a Database-profile configuration, and
`stale_service_is_marked_and_rebuilt_in_the_background` passes one carrying
an adjustable clock. See "Testing a cached route" in
[RenderCache Operations](render-cache-operations.md).

Two ordering rules, both the caller's responsibility:

1. Every route and group must be opted in **before** `install`, which reads
   whatever has been registered by that point.
2. `install` appends to the global middleware chain, so it must run **after**
   the session, locale, and identity middleware whose request-scoped state
   the cache middleware reads while deriving a lookup key.

The install fails closed, through two probes. It checks that the tables the
configuration would reach exist, and it pings every distinct Redis endpoint
the configuration would use, once per endpoint. Either failing stops the
boot with one actionable sentence naming the migration or the variable to
fix, so nothing is ever served against a missing table or an endpoint
nothing answers. (Live's own instance ledger is probed separately, by
`Server::run`, before any request is served.)

## Choosing where a route's entries live

The profile decides what L1 *is*; the policy decides which routes use it.
The builder stores in L0 only unless a route declares otherwise, so a shared
tier is populated on purpose:

```rust
RenderCachePolicy::builder(RepresentationClass::PublicShared)
    .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
    .layers(StorageLayers::l0_and_l1())
    .build()?
```

`the_database_profile_serves_a_hit_through_the_sql_stores` proves the round
trip end to end on the Database profile: the published entry is read back
out of the SQL store under the key the middleware derived, then L0 is
emptied and the next request is still answered without a render. See
[RenderCache Representations](render-cache-representations.md) for how to
decide per route.

## One conformance suite, every provider

Every store answers the same suite.
`framework/tests/render_cache/store_conformance.rs` runs the engine's
provider scenarios - written against the `RenderStore`
trait alone - over the file-backed L1, the SQL L1 on SQLite, PostgreSQL, and
MySQL, and the Redis L1, and the in-process store answers the same suite in
the engine crate. PostgreSQL, MySQL, and Redis run through ignored tests
that `scripts/check-postgres.sh`, `scripts/check-mysql.sh`, and
`scripts/check-redis.sh` select by name against real servers. A provider is
not "supported" here because it exists; it is supported because it passes
the same words as every other one.

## What the tiers promise, and what they do not

- **No cross-node waiting.** A key another node is already rebuilding is a
  bypass: this node renders and publishes nothing. Bounded duplicate
  computation across nodes is accepted; two accepted publications are not,
  and the store's own publication fence is what forbids the second.
- **A lost backend is a miss, never a wrong answer.** Eviction, expiry, or a
  Redis restart makes entries miss and instances missing. The coherence
  check against the database generation ledger runs on every hit whatever
  served the bytes.
- **Tampered bytes are a miss.** Entry bytes in a row or a hash are a signed
  codec frame, so a torn, truncated, or altered value fails its integrity
  check and is treated as a miss rather than served.
- **The database and Redis tiers do not evict to make room.**
  `RENDER_CACHE_L1_BYTES` bounds one entry there, not the table or the
  keyspace; growth is bounded by retention instead. Only the file tier
  bounds a whole directory, because it owns that directory alone.
- **The Redis adapters target a single Redis 7 or newer instance.** Redis
  Cluster is refused: the scripts touch keys they do not declare, and they
  read the store clock with `TIME` inside a script.
- **MySQL needs 8.0.19 or newer** for precise duplicate-key classification.
  Older MySQL and MariaDB report a collision this build will not attribute
  to a table, so it degrades to a provider-unavailable error - the safe
  direction, and nothing is granted twice either way.
- **Every cross-node expiry decision is made on the backend's clock**, read
  inside the operation that acts on it. A node whose clock runs fast can
  neither extend a lease, nor hide a live entry from its peers, nor declare
  a peer's record elapsed.

### Why Suprnova diverges

Laravel's response-caching packages inherit the cache store you already
configured, so "deploying the cache across several nodes" means pointing
`CACHE_STORE` at Redis and trusting that whatever is in there is right.
There is no separate notion of who may rebuild an entry, no fence that stops
two workers publishing conflicting bytes for the same key, and - most
consequentially - no authority underneath the store. If Redis holds a page,
the page is served; if Redis is flushed, everything is recomputed. The store
*is* the truth.

Suprnova splits the two apart deliberately. The shared tier holds bytes and
nothing else; the database holds generation truth at every profile, and it
is what a hit is checked against. That is why losing Redis here costs
latency rather than correctness, why a rebuild is leased and a publication
is fenced rather than raced, and why the same route declarations run
unchanged from `cargo run` on a laptop to a database-coordinated fleet. The
cost is a migration your application has to list and a database read on the
hit path that a plain key-value cache does not pay - one statement, or zero
under a validation lease. See
[RenderCache Generations](render-cache-generations.md) for what that read
buys.

## Next

- [RenderCache Operations](render-cache-operations.md) - the console
  commands, telemetry, disk hygiene, and what to do when something is wrong
- [Deployment](deployment.md) - the surrounding production checklist
- [Migrations](migrations.md) - how the `Migrator` list above is applied
