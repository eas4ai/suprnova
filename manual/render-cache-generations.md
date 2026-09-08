# RenderCache Generations

Most caches expire. RenderCache expires too, but expiry is the backstop
rather than the mechanism. The mechanism is a **generation**: every piece of
data a render read has a counter in the database, the render stores the
counters it saw, and a write advances the counter for whatever it changed.
A stored representation is current when the counters it saw still match the
counters the database holds now. You write no invalidation rule for your own
data, because an ordinary `model.save()` already is one.

This chapter is about that machinery from the outside: what a render is
recorded as depending on, how coarse those dependencies really are, what the
framework cannot see and therefore cannot invalidate, how the coherence
check is paid for on a hit, which request rebuilds when several want the
same entry at once, and what a visitor is served in the window between "no
longer current" and "rebuilt". Every claim below is held down by
a named test or a checked-in measurement; the dogfood examples are routes in
`app/src/live/mod.rs` proved by `app/tests/live_render_cache.rs`.

## What a render is recorded as depending on

While a render runs, a request-scoped collector records each dependency it
can name: a table read, a record read by primary key, a query
class, a relation, a configuration identity, a feature, a locale, a route,
and one always-present `Broad` identity that every representation observes.
Reads through the ORM and the query builder record themselves; you write
nothing.

`/live/todos` is the whole pattern in one handler:

```rust
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView { count: titles.len(), titles })
    }
    .await;
    result.map_err(failed)
}
```

`Todo::all()` records the `todos` table. Nothing else in the handler or the
template reads the session, the signed-in visitor, or a translation, which
is what lets the route stay a shared representation at all.

## An ordinary write is the invalidation

`an_orm_write_invalidates_the_todos_document_through_generations` walks the
whole cycle through the running application:

1. The first `GET /live/todos` renders and publishes.
2. The second is a hit: it never reaches the handler, carries no `Warning`,
   and schedules nothing.
3. A `POST /todos/random` writes a row - through the application's own
   route, with the session and CSRF token a browser would send.
4. The next `GET` is served with `Warning: 110 - "Response is Stale"` and
   schedules exactly one background rebuild. Its five fresh minutes have
   barely started, so the advanced generation of the `todos` table is the
   only thing that can explain either.
5. That rebuild really runs: the test waits on the render counter - a state
   barrier, not a timed wait - until a render the test itself did not
   dispatch has happened.
6. The written row really is in the listing. This is a **separate** step and
   deliberately not an assertion about the background rebuild's own output:
   the test drops L0 first and renders again, because the rebuild's
   publication lands at a moment nothing reachable from the application
   makes observable, so asserting on whichever request happened to catch it
   would be a race.
7. And the route settles back to a plain hit against the republished entry.

No cache key was named anywhere in that sequence. An ORM write inside a
`DB::transaction` advances its generations inside that same transaction, so
a rolled-back write advances nothing at all.

## Invalidation is table-granular today

This is the single most important thing to know before you size a cached
route.

A point read through the ORM records the **table** identity as well as the
row identity. `Model::find` calls `observe_table_read(Self::TABLE)` before
it looks anything up and `observe_record_read_json` after it hydrates a row,
so an entry that read one row depends on the whole table. Any write to that
table therefore invalidates **every** cached entry that read from it, not
just the entries that read the row that changed.

That is safe - it can only invalidate too much, never too little - and it is
measured rather than assumed. The invalidation-storm workload in
`framework/benches/render_cache_workloads.rs` publishes 64 keys over 12
record identities, drives 1,000 writes, and records the fan-out it observed
in `crates/suprnova-live/benchmarks/render-cache-workloads-v1.json`
(abridged; the recorded object also carries the burst, sweep, hit, rebuild,
statement, and latency fields):

```json
"invalidation_storm": {
  "keys": 64,
  "identities": 12,
  "writes": 1000,
  "every_write_invalidates_every_key": true,
  "rebuilds_per_write": 1.28,
  "final_bodies_coherent": true
}
```

Design around it. A cached route backed by a table your application writes
to constantly will rebuild constantly, whatever its freshness window says. A
cached route backed by a table that changes when an editor publishes
something will sit still for hours. If you need finer granularity than the
table, the honest answer today is that you do not have it.

## What the framework cannot see

A dependency that cannot be named cannot be invalidated, and the framework
is deliberate about which of those it declines to store and which it lets
through.

**Declined outright.** Raw SQL through `DB::select`, `DB::select_one`,
`DB::scalar`, or `DB::select_on` cannot name the tables its statement read,
so the render is marked unobservable and never stored. The response is still
served, correctly, every time. The framework's own RBAC role and permission
checks read this way, so a cached route that evaluates one never stores.
Reads through `DB::table(..)` know their table and cache normally.

**Invisible, and your responsibility.** A request header read through
`Request::header`, a `Config::get` call, and an Eloquent global scope that
filters a query from its own per-request state all change what a render
produces without the collector seeing anything. Declare the matching
variance dimension on such a route; nothing here can catch the omission for
you.

**Known gaps.** Nothing advances a generation for a feature flag when the
flag changes, and a write made by a queue worker, a scheduled task, or a
console command advances nothing at all, because only the process that ran
`RenderCache::install` carries the write-side instrumentation. A page
depending on such a write stays current only within its freshness window;
run `render-cache:epoch-advance` after a job that changes what cached pages
show. See [RenderCache Operations](render-cache-operations.md).

## What a hit costs

The coherence check is what turns "we have bytes" into "these bytes are
current", and it is the only work a hit does.

A hit runs **no handler, no ORM query, no template, and no serializer**, and
copies no body bytes: the bytes the server writes to the socket are the
bytes the store holds, proved by address rather than by value in
`framework/tests/render_cache/bypass.rs`. What is left is the database read
that proves currency, and how often you pay for it is the policy's
`CoherenceMode`:

| Mode | SQL statements per hot hit | What it trusts |
|---|---|---|
| `Authority` (default) | exactly 1 | the ledger, reread every hit |
| `Lease { max_age_ms }` | 0 | a locally granted validation lease, until it expires |

`an_authority_mode_hit_issues_exactly_one_statement` holds authority mode to
one round trip: the observed generations and the authority epoch are read
together in a single `UNION ALL`, not as two reads.
`a_lease_mode_hit_runs_nothing_and_issues_no_statement` holds lease mode to
zero, because the epoch the key was derived under is leased alongside the
generations rather than read per request.

The epoch itself is read once per process, not once per request.
`the_epoch_is_read_once_at_first_use` measures the first miss of a fresh
runtime against an otherwise identical second one and finds the first pays
exactly one statement more - the single authority read that fills the epoch
lease. Every request after that pays nothing for it.

## When the epoch moves

`render-cache:epoch-advance` is the emergency invalidation, and the epoch is
baked into every lookup key, so what happens next depends on where you are
standing:

- **On the node that ran the command**, the very next request sees the new
  epoch. L0 is cleared outright at the same instant, and the cache is
  invalidated immediately.
- **On another node**, an `Authority`-mode route learns at its very next
  hit. A `Lease`-mode route learns at its next authority reread, which is at
  most `max_age_ms` later.

A route with a stale-servable window serves the moved entry once under
`Warning` while the rebuild runs behind the request; a route without one
rebuilds in the foreground and the requester waits for it. That difference
is the whole reason to declare a stale-servable window, and it applies to
any move, not only an epoch advance.

Three tests in `framework/tests/render_cache/middleware.rs` hold those
paths down by name:
`an_epoch_advanced_by_another_node_reaches_an_authority_mode_route_on_its_next_hit`,
`an_epoch_advanced_by_another_node_reaches_a_lease_mode_route_when_its_lease_expires`,
and
`an_epoch_advanced_by_another_node_serves_a_stale_servable_entry_once_then_rebuilds`.

## One rebuild per key: singleflight and waiters

When an entry is missing or no longer current, the requests that arrive for
it do not all render. They are admitted through a **rebuild coordinator**,
which picks exactly one of them:

- The **leader** is the one request that renders and may publish. It holds a
  lease on that key for the length of its render.
- **Waiters** are the requests that arrive for the same key while the leader
  holds it. They wait in process, and when the leader releases they
  re-evaluate what is now stored and serve that. A waiter never trusts the
  wait: if the leader's cycle failed to publish, or published something the
  waiter's own freshness check finds dead, the waiter renders too, rather
  than serving what it found. `a_singleflight_waiter_never_serves_a_superseded_entry_as_fresh`
  in `framework/tests/render_cache/middleware.rs` is that rule.
- A request that arrives once `RENDER_CACHE_MAX_WAITERS` (default 128) are
  already waiting **bypasses**: it renders and publishes nothing, rather
  than growing an unbounded queue.

`concurrent_misses_render_once_and_waiters_reuse_the_publication` proves the
ordinary case end to end - two concurrent misses, one render, identical
bodies - and
`one_leader_per_key_and_fence_with_bounded_waiters` in
`crates/suprnova-live/tests/render_cache_singleflight.rs` proves the cap
directly against the coordinator: past its waiter limit, admission answers
`Bypass`.

Two publications for one key can never both be accepted, whatever the
coordinator decided. A leader mints a publication token under its lease, and
the store compares that fence before it writes: an older epoch, or an equal
epoch with a lower token, loses. That is what makes duplicate *rendering*
safe to accept while duplicate *publishing* is not, and it is why there is
no cross-node waiting at all - a key another node is rebuilding is a bypass
here. See [RenderCache Deployment](render-cache-deployment.md).

## Serving something while it is rebuilt

The four freshness states, the bands `FreshnessPolicy` sets, and the
`Warning` and `Age` a stale response carries are defined in
[RenderCache Representations](render-cache-representations.md). What matters
here is that a generation move puts an entry into those bands early: a
moved entry is evaluated at an effective age of at least its fresh interval,
so on a route with a stale-servable window it lands in that band and is
served once under `Warning` while the rebuild runs behind the request. That
is exactly what step 4 of the write test above observes, on an entry whose
five fresh minutes had barely started.

`stale_service_is_marked_and_rebuilt_in_the_background` shows the same
handoff driven by the clock rather than by a write: past `/live/todos`'s
300,000 fresh milliseconds and inside its 60,000 stale-servable ones, the
visitor is handed the copy on hand under `Warning: 110 - "Response is
Stale"` and `Age: 300`, exactly one rebuild is scheduled, and that rebuild
really runs.

The stale-on-error fallback covers the request that leads a rebuild **and** a
waiter behind a leader whose rebuild failed. Both are answered the same way:
the stale bytes under `Warning`, rather than the failure.
`framework/tests/render_cache/races.rs` proves each arm separately -
`a_waiter_behind_a_failed_leader_is_served_the_stale_entry_it_was_waiting_on`
and `a_waiter_that_re_evaluates_onto_a_stale_on_error_entry_falls_back_to_it` -
and the second one by revert: dropping the fallback from the waiting arm
turns its final assertions from `200` into `500`.

Stitched routes are the exception, and it is a deliberate one. A `Composite`
entry is never served by the stale-on-error fallback and never triggers a
background rebuild: serving a stored shell after a failed rebuild would
answer a request the route's own authorization chain never got to gate, and
a background rebuild carries none of the request's authorization state, so
its shell would be whatever the page renders for nobody. On a stitched
route, a failed rebuild's own outcome is what the client sees.

## Cached routes are read paths

The leader's render runs inside a database transaction, opened at
`REPEATABLE READ` on PostgreSQL and MySQL, so the generations it records and
the data it read share one snapshot. Two consequences follow.

A cached route's handler that **writes** competes with concurrent writers
for the same rows, and on PostgreSQL a handler that updates a row another
transaction changed after the render began sees a serialization failure.
Design cached routes as read paths.

A write made outside any transaction - `model.save()` on its own - commits
its row first and advances its generations in an immediately following
transaction. The moment between the two is "new data, old generation": it
costs one extra rebuild and never serves stale content.

Finally, after the render finishes, the observed dependencies and the epoch
are read again, outside the render's own transactional view. Anything that
moved during the render discards the candidate rather than publishing it.
That is why a write landing mid-render costs a rebuild instead of a wrong
page.

### Why Suprnova diverges

Laravel's cache is a key-value store and its response-caching packages are
built on top of it, so invalidation is something you write. You call
`Cache::forget`, or you tag entries and flush a tag, or you register a model
observer that clears the keys you believe that model feeds. Every one of
those is a mapping you maintain by hand, and the failure mode is silent: the
page that nobody remembered to forget keeps serving until its TTL runs out.

Suprnova inverts the direction. The render records what it read, the write
advances what it changed, and the two meet in a database ledger rather than
in your head. There is no `forget` call to forget. The price is that the
recorded dependency is a table rather than a row, so a busy table rebuilds
its dependents often, and that reads through raw SQL are declined from
storage rather than cached with a dependency nobody can name. Both of those
are visible and measured - the fan-out in the checked-in storm workload, the
decline in your own missing `Age` header - rather than a stale page you find
out about from a customer.

## Next

- [RenderCache Deployment](render-cache-deployment.md) - profiles,
  providers, and the migration that makes generation truth durable
- [RenderCache Representations](render-cache-representations.md) - what is
  actually stored, and under what key
- [Database](database.md) - transactions and isolation, which cached renders
  run inside
