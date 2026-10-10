# Data cache

Status: Draft
Prefix: QCACHE

Drafted 2026-10-05 from the developer's original cache design, which he
described before RenderCache was built and which RenderCache did not
implement. His words (2026-10-05, 12:33): "I never presented that data and
html would get cached together. I described html like Smarty templates
worked and I described data caching as using a hash of the result that got
deleted when the data changed by a data operation (insert, update, delete,
etc). In my old system, I kept a hash of the query and a list of the
tables, field associated with the query... in a serialized file. I saved
them with the route that generated them." And (12:36): "The route works
like a trigger to check the cache." And (12:37): "So 2 triggers... database
operations that touch those tables delete all cached data associated with
them." This spec records that design; it is
the cache that serves pages a nonce CSP keeps RenderCache from storing
(SEC-005), and it serves every other page as well.

## Observed at 2bd4bd53d

Status: Observed

- RenderCache stores the rendered representation and checks, on every hit,
  that the generation counters it recorded still match the database's
  (`manual/render-cache-generations.md`; `framework/src/render_cache/middleware.rs:3305-3319`).
  Data and markup are one cached unit, so a response whose CSP carries a
  nonce is declined (CACHE-004; `middleware.rs:2789-2798`).
- The collector records what one render read: tables, and rows for
  primary-key point reads (`framework/src/render_cache/collector.rs:48-53,76-80`;
  `framework/src/eloquent/model.rs:633,739`). It records no columns and no
  query identity, and exists only inside a RenderCache scope
  (`collector.rs:509-511`).
- Writes advance the counters of what they changed (the generation
  machinery), so the framework already knows, at write time, which tables
  and rows a write touched.
- Askama compiles every template at build time, so the template half of the
  developer's design (a template cached apart from its data) exists in that
  form.

## Requirements

[QCACHE-001] The framework MUST cache query results apart from any
rendering: a result is stored under the hash of the query (its SQL,
bindings and connection) together with the tables and the fields the
query read, and rendered markup is never the cached unit.
Falsifier: a cached result is stored with or keyed by markup, or a cached entry lacks its tables and fields.
Mechanism: `data-cache`.
Rationale: The developer's design: data and HTML are never cached together.
Status: Draft

[QCACHE-002] Two triggers drive the cache. The write trigger: an insert,
update or delete that touches a table MUST delete every cached entry
associated with that table when the write commits; a write inside a
transaction that rolls back deletes nothing. The read trigger is the route
(QCACHE-003).
Falsifier: a stale result is served after a committed write to a table it read; an entry listing an untouched table only is deleted; or a rolled-back write deletes an entry.
Mechanism: `data-cache`.
Rationale: The developer, 2026-10-05 12:37: "database operations that touch those tables delete all cached data associated with them." Invalidation on write, not a check on read, so a hit costs no database statement.
Status: Draft

[QCACHE-003] The route is the trigger: a route keeps the record of the
queries it ran, and a request to that route checks each by its hash. A hit
MUST serve the stored result without running the query; a miss MUST run
the query and store the result again. The record MUST follow the route
across restarts.
Falsifier: a request to a route with every entry present runs a query; a request after a deleting write serves the deleted result or fails to re-cache; or a restart loses the route's record.
Mechanism: `data-cache`.
Status: Draft

[QCACHE-004] A query the framework cannot attribute to tables and fields,
a query inside an open write transaction, and a query on a connection the
framework does not track MUST NOT be cached. Reading a result MUST never
return rows a write has deleted from the cache.
Falsifier: an unattributable, in-transaction or untracked-connection query is cached, or a request observes a result between its deletion and its rebuild.
Mechanism: `data-cache`.
Status: Draft

[QCACHE-005] The cache MUST have the same driver choice as RenderCache's
storage (a trait with drivers), and the manual MUST document it with a
"Why Suprnova diverges" section: Laravel has no query cache, only
`Cache::remember` around a query by hand.
Falsifier: the cache is tied to one storage backend, or the manual omits it.
Mechanism: `data-cache`, `manual-check`.
Status: Draft

## RenderCache

[QCACHE-006] RenderCache MUST stay the page cache it is (CACHE-001 to
CACHE-010), and the data cache MUST sit beneath it: a RenderCache rebuild's
queries are served by the data cache, a page RenderCache declines (a nonce
CSP among the reasons) is served from it, and the write trigger
(QCACHE-002) MUST use the same report of touched tables that advances
RenderCache's generations, so neither cache sees a write the other misses.
Falsifier: a RenderCache rebuild bypasses the data cache; a declined page's queries bypass it; or a write that advances a generation leaves a data-cache entry for that table.
Mechanism: `data-cache`, `render-cache`.
Rationale: Of the two options (RenderCache kept with the data cache beneath it; the data cache replacing RenderCache's coupled unit), the developer ruled "a" on 2026-10-05: RenderCache is shipped and Agreed, and the data cache gives his design for the data without removing it.
Status: Draft
