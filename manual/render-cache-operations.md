# RenderCache Operations

A cache you cannot see is a cache you cannot trust. RenderCache is built so
that the two questions an operator actually asks - "is this page being
served from a stored copy?" and "how do I make it stop?" - have direct
answers that do not involve reading a body out of a store or guessing at a
key. There are two console commands, six telemetry counters, one bounded
disk sweep, and one emergency lever.

This chapter is the operating surface: the commands and exactly what they
print, the counters and their closed outcome sets, how the file tier
reclaims disk, what to do when something is wrong, and how the cache's own
performance is measured and what those numbers are honestly worth. The
command examples are the ones
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
prints `no entry (current epoch: {epoch})` when the key names nothing
stored, and it fails - it does not report success - on an unparseable key or
with no runtime installed.

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
  Nothing stale can be proven current, because the coherence check runs
  against the database generation ledger whatever served the bytes.
- **A rebuild leader died mid-rebuild.** Its lease is taken over once store
  time passes the expiry, and the former leader's own publication is fenced
  out rather than racing the new one. It publishes nothing; its request's
  response is still served.
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
- [Deployment](deployment.md) - the production checklist around them
