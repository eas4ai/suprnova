# RenderCache Operations

A cache you cannot see is a cache you cannot trust. RenderCache answers two
operator questions directly and without ever printing a stored page: **what
is this node holding under this key, and is it still current?** and **how do
I make everything stop?** It answers a third - "is this route being served
from a stored copy at all?" - through telemetry and through the `Age` header
rather than through a command, because that question is about traffic rather
than about one entry. There are two console commands, six telemetry
counters, one bounded disk sweep, and one emergency lever.

This chapter is the operating surface: the commands, exactly what they print
and what they can see; the counters and their closed outcome sets; how the
file tier reclaims disk; how to test a cached route so the test proves
caching rather than merely responding; what to do when something is wrong;
and how the cache's own performance is measured and what those numbers are
honestly worth. The command examples are the ones
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` drives
through this repository's own console entry point in
`app/tests/live_render_cache.rs`.

## The two console commands

Both are hidden commands, registered by the framework and reachable through
your project's `console` binary like any other. Neither ever prints a stored
body or a raw dependency identity.

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** reports one stored entry's shape: its
representation class, its `body_bytes`, its other metadata, and the current
authority epoch beside it, so you can tell whether the entry you are looking
at is still live authority or has already aged out from underneath. It
prints `no entry (current epoch: {epoch})` when the key names nothing it can
see, and it fails - it does not report success - on an unparseable key or
with no runtime installed.

**It reads this process's in-process L0 and nothing else.** `RenderCache::inspect`
looks the key up in L0 alone; it never consults the L1 tier. On the Database
or Redis profile that matters: an entry that is live in
`suprnova_render_entries` or in Redis, published by another node or by this
one before a restart, prints `no entry` here unless this process has served
it since it started. Read the report as "what this node has in memory",
never as "what the deployment has stored". The same is true of
`RenderCache::store_inspection`, which reports L0 occupancy and the current
epoch.

The key is the text the lookup itself uses: `rk1.` plus 43 base64url
characters, which is what your application's logging and telemetry can
surface. It is not a second hash of anything, so a key an operator holds
names exactly one entry.

That body-free claim is checked, not merely stated. The test takes the
document that was actually served, splits it into lines, and requires that
**every** non-trivial line of it is absent from what the inspect report
printed.

**`render-cache:epoch-advance`** is the emergency invalidation. It advances
the authority epoch and prints `epoch advanced to {epoch}`. Because the
epoch is baked into every lookup key, every currently stored entry becomes
unreachable by ordinary lookup at its next request - immediately, with
nothing to enumerate and nothing to delete. The in-process tier is cleared
outright at the same instant. The test asserts the printed line and then the
consequence that matters: after the command, the route renders again.

Reach for it when something is wrong with cached content and you cannot wait
for individual entries to expire, and after a job that changed what cached
pages show (see "known gaps" in
[RenderCache Generations](render-cache-generations.md)).

## Permission changes

`RenderCache::bump_permission_version().await?` is the one invalidation call
an application makes by hand, and it is not really an operations command -
it belongs in the code path that changes what a signed-in user is allowed to
do. It advances a persisted generation that every principal-keyed render
observes, it survives a restart, and it joins the transaction the role
change runs in when there is one. Without it, a user whose permissions just
changed keeps matching whatever was cached under their prior permission set.

## Telemetry

Six closed counter names, and nothing in any of them names a tier, a
provider, or a backend:

| Counter | Attribute |
|---|---|
| `suprnova.render_cache.lookups` | `outcome` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | none |
| `suprnova.render_cache.rebuilds` | none |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |

`lookups` and `hits` carry the same closed set of eight outcomes:

- `l0`, `l1` - a fresh entry served from the in-process or the shared tier.
- `conditional` - a fresh hit whose `If-None-Match` matched, answered `304`.
- `stale` - a stale-servable entry served immediately, or the stale-on-error
  fallback after a foreground rebuild failed.
- `miss` - nothing found, a stale-on-error rebuild in progress, or a dead
  entry.
- `bypass` - an undeclared query parameter, an unresolvable declared
  variance dimension, or an exhausted waiter list.
- `moved` - the reread after rendering found a dependency or the epoch had
  changed; the candidate was discarded, never published.
- `declined` - the render was not storable: eligibility, an overflowed
  observation report, an `Uncacheable` classification, a Live document rule,
  or a bound.

`hits` increments only for `l0`, `l1`, `conditional`, and `stale`.
`publications` counts only a store answering "published", never a fenced or
rejected attempt. `rebuilds` counts one per spawned background rebuild.

The two stitch counters carry their own sets: `assembled` and
`fail_document` for assemblies; `rendered`, `omitted`, `fallback`, and
`failed` for slots.

**A high `declined` rate is the signal worth alerting on.** It means routes
you opted in are rendering and serving correctly while never being stored,
and the response looks identical either way. The fastest local check is two
requests in a row: if the second carries no `Age` header, nothing was
stored.

## Disk hygiene

Only the file tier needs sweeping, and it mostly sweeps itself.

`FileRenderStore` stores one file per key, flat under `RENDER_CACHE_L1_DIR`.
An entry is dead when its age since publication reaches the retention it was
published with, or when its fence epoch is older than the current epoch.
Retention comes from the same class-aware dead edge the live freshness check
uses, so a private entry's file is retired earlier than a public one's and a
sweep can never disagree with a freshness check about whether an entry is
truly dead.

`sweep` removes at most 64 entries per call, oldest publication first, and
returns whether more remain. It runs automatically on every 256th
publication, so a healthy directory needs no attention. `RenderCache::sweep()`
drives it explicitly when you want to, and a backlog larger than one call's
limit drains across later triggers rather than blocking on one long scan.

Two things the sweep is not:

- **An epoch advance does not touch L1.** It clears L0 outright, because
  that is in-process memory with nothing to reconcile against, and leaves
  every pre-epoch file on disk until a sweep reclaims it. That is disk
  hygiene, not a correctness concern - the files are already unreachable by
  lookup.
- **The database tier has no automatic sweep**, and is reclaimed only
  through `RenderCache::sweep()`. The Redis tier needs none: every entry it
  stores carries an expiry and Redis reclaims the bytes itself.

Publication is crash-safe. It writes a temporary file, fsyncs it, renames it
over the target, and fsyncs the parent directory, so a reader only ever sees
the previous complete file or the new complete file. On open, the store
removes any leftover temporary file and any file that fails its frame check,
treating a torn write as self-healing rather than a permanently poisoned
entry.

## Testing a cached route

A test that asserts a cached route responds correctly passes whether the
response came out of the store or out of a fresh render. Every claim has to
be made against something only a stored entry actually being served can
produce. Four patterns do that, and this repository's own dogfood tests use
all four: `app/tests/live_render_cache.rs` with the harness in
`app/tests/live_support/mod.rs`.

**1. Count renders on the handler side of the cache.** Register a counting
middleware *after* `RenderCache::install`. Registration appends, so it lands
closer to the handler than `RenderCacheMiddleware`, and a request the cache
answers returns before calling it:

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// After the install, so it only sees requests the cache forwarded.
render_counter::register();
```

The difference between two readings of `render_counter::renders()` is then
the number of renders the cache did not avoid, and nothing else - unlike
identical bodies or an `Age` header, both of which have honest non-cache
explanations. Every hit assertion in
`an_orm_write_invalidates_the_todos_document_through_generations` rests on
it. Wait for a render you did not dispatch (a background rebuild) with the
counter's own barrier, `wait_until_renders_at_least`, never with a sleep.

**2. Read the entry back.** Two facade calls are ordinary public API:
`RenderCache::store_inspection()` reports L0 occupancy, bytes, and the
current epoch, and `RenderCache::inspect(key_text)` reports one entry's
body-free metadata. Alongside them the framework exposes hidden test seams -
`#[doc(hidden)]`, and named `_for_test` so nothing mistakes them for
application API:

| Seam | What it gives a test |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | the same key text the middleware derives |
| `RenderCache::inspect_route_for_test(pattern)` | that key's L0 entry: class, kind, status, `body_bytes`, slots |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | the same, out of the configured L1 tier |
| `RenderCache::clear_l0_for_test()` | empties L0 and leaves L1, the epoch, and the coordinator alone |

`the_public_document_is_a_hit_whose_seed_still_promotes` uses
`store_inspection` and `inspect_route_for_test` to assert the entry exists
and is stored under the declared class;
`the_database_profile_serves_a_hit_through_the_sql_stores` uses
`inspect_l1_for_test` and then `clear_l0_for_test`, which is the only way to
prove a later request came out of L1 rather than out of memory.

**3. Move the clock instead of waiting.** The clock the runtime reads is
settable on a `RenderCacheConfig` and never by `from_env`, so a test that
needs a freshness band installs its own:

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Bound to its own name first: passing `Arc::clone(&clock)` inline leaves
// the compiler inferring the trait object as the clone's return type.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... install through the application's own configuration seam, then:
clock.advance_ms(300_001);
```

`AdjustableTestClock` comes from `suprnova::live::testing`, and `unix_now_ms`
is the harness's own wall-clock reading, so an adjustable clock starts where
the system one is rather than at an epoch the rest of the process would
disagree with. The harness wraps the pair as `setup_app_with_clock` and
`advance_clock_ms`, the second of which panics rather than silently doing
nothing when the boot took the system clock.
`stale_service_is_marked_and_rebuilt_in_the_background` is the test.

**4. Count SQL statements.** A cache that skipped the handler but still
consulted the database on every hit satisfies every handler-side counter and
still costs a round trip. `DbConnection::observe_statements_for_test` points
SeaORM's metric callback at a counter of your own, and it sees statements on
the pool and on every transaction started from it:

```rust
// Immediately after connecting, before the connection is cloned or bound
// into the container: installing needs sole ownership of the pool, and the
// call reports `false` rather than counting nothing silently.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

The callback is told nothing about the statement - no SQL text, no bound
value - because a count is the whole point.
`framework/tests/render_cache/bypass.rs` is written entirely on this
pattern: `a_lease_mode_hit_runs_nothing_and_issues_no_statement` holds a
lease-mode hit to zero statements, `an_authority_mode_hit_issues_exactly_one_statement`
holds an authority-mode hit to one, and
`the_epoch_is_read_once_at_first_use` measures two misses against each other
to show the epoch costs one read per runtime.

Two habits worth keeping. Boot the harness through your own application's
configuration seam rather than through a hand-built router, so the test
installs the same routes, policies, and middleware ordering the server does.
And never add a timing wait: every barrier above is a state barrier on a
counter, which is what makes these tests reproducible rather than flaky.

## When something is wrong

- **A page is showing content you know is old.** Check whether the route is
  storing at all (two requests, look for `Age`). If it is, and the write
  that should have invalidated it came from a queue worker, a scheduled
  task, or a console command, that write advanced nothing: run
  `render-cache:epoch-advance`.
- **A page you expected to cache never carries an `Age` header.** It is
  being declined, not failing. Work through the classification list in
  [RenderCache](render-cache.md): a session read, an identity read on a
  route with no `Principal` variance, a locale read with no `Locale`
  variance, an authorization check, or a raw SQL read.
- **A backend is unreachable.** `RENDER_CACHE_FAILURE` decides: `open` (the
  default) serves the route uncached, `closed` answers a bare `503`. A
  backend missing at boot stops the boot instead, with a sentence naming the
  migration or the variable to fix.
- **Redis was flushed or restarted.** Entries miss and are re-rendered.
  Nothing stale can be proven current: currency is proved against the
  database generation ledger, never against the tier that held the bytes.
- **A rebuild leader died mid-rebuild.** Its lease is taken over once store
  time passes the expiry, and the former leader's own publication is fenced
  out rather than racing the new one. It publishes nothing; its request's
  response is still served.
- **An L1 file was torn by a crash or a full disk.** Nothing serves it. Each
  file carries a digest over its own frame, so a truncated or altered file
  fails that check and is a miss; the store removes it, and any leftover
  temporary file, the next time it opens. A torn write is self-healing here
  rather than a permanently poisoned entry.
- **The database was restored from a backup.** The generation ledger is the
  authority every hit is proved against, so restoring it changes what
  "current" means, and two facts decide the outcome. The coherence
  comparison is an inequality in **either** direction, so any stored entry
  whose observed generations differ from the restored ledger's is treated as
  moved and rebuilt rather than served. And the authority epoch is part of
  every lookup key. So: run `render-cache:epoch-advance` before the node
  serves - it fails loudly rather than reporting success if the epoch
  singleton is missing, which is also how you find out the migration did not
  come back with the data - and then empty the shared L1 tier by hand (the
  file directory, or `suprnova_render_entries`) rather than waiting for the
  sweep, whose epoch clause only reclaims entries stamped with an epoch
  *older* than the current one. L0 needs nothing: the epoch advance clears
  it outright.
- **You need everything gone, now.** `render-cache:epoch-advance`.

## Measuring it

RenderCache ships two benchmarks, and they are **on-demand tools, never gate
steps**:

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

That runs the engine bench (`render_cache_budget`, the hot-hit and composite
assembly measurements with a counting allocator), then the framework
workload bench (`render_cache_workloads`, the same route through the whole
middleware), then the contract test over the checked-in results. Both are
pinned to `SUPRNOVA_LIVE_S1_CPUSET`.

A full run needs a disposable PostgreSQL (`PG_TEST_URL`) and a disposable
Redis (`REDIS_TEST_URL`), because the checked-result contract requires all
three recorded profiles. **A partial run must redirect both result files**
with `SUPRNOVA_LIVE_BENCH_RESULT` and `SUPRNOVA_LIVE_WORKLOADS_RESULT` under
`benchmarks/local/`; without that it overwrites the checked-in results with
a shorter file and then fails its own contract.

The checked-in numbers, from
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` and
`render-cache-workloads-v1.json`:

| Measurement | Value |
|---|---|
| Engine work for a fresh `Complete` L0 hit, p95 | 0.76 microseconds |
| Heap allocations, fresh hit | 3 |
| Heap allocations, conditional `304` hit | 3 |
| Heap allocations, hit bounded by a seed deadline | 4 |
| Body copies on any of them | none; the buffer is shared |
| The same route through the middleware, server side, p95 | 14.4 microseconds |
| The same request over a loopback HTTP round trip, p95 | 109 microseconds |
| SQL statements per hot hit (lease mode) | 0 |

The middleware figures are for a 65,536-byte body whose render read 12 rows,
recorded as 14 observed dependency identities.

**Read those as exploratory, not as qualified evidence.** Every checked-in
result carries `"classification": "local_exploratory"` and
`"s1_requirements_met": false`: they were produced on a developer
workstation with a shared CPU, a `powersave` governor, and loopback
providers. They are useful for catching a regression of a whole order of
magnitude and for nothing finer. A number is qualified evidence only when it
was produced on the dedicated runner with its attestation set, and these
were not.

### Why Suprnova diverges

Laravel's response-caching packages leave operations to the cache store
underneath. Inspecting an entry means finding its key by hand and reading
the value - which is the rendered page, so looking at it means printing
somebody's HTML to a terminal - and invalidating everything means flushing a
store that also holds your sessions, your rate limits, and your queue.
Observability is whatever the store driver happens to emit.

Suprnova gives the cache its own operating surface, deliberately narrow.
Inspection is body-free by construction, so an operator can confirm an entry
exists, what class it is stored under, and how large it is, without ever
being shown its contents. Invalidation is an epoch bump that costs nothing
to apply and touches only this cache - your sessions and your queue are not
in the blast radius. Telemetry is a closed set of six counters with closed
attribute sets, which is what makes a dashboard over them stable across
releases rather than a set of strings that drift. The trade is that there is
no "delete this one key" command: the levers are per entry read-only, or
epoch-wide.

## Next

- [RenderCache](render-cache.md) - the declarations these commands operate on
- [Observability](observability.md) - where the counters above are exported
- [Testing](testing.md) - the surrounding test conventions the patterns above
  sit inside
- [Deployment](deployment.md) - the production checklist around them
