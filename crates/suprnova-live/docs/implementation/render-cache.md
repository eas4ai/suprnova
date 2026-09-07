# RenderCache

RenderCache is Suprnova's response cache for GET and HEAD routes: a route or
group opts in with a declared policy, the framework middleware decides
whether a request can be answered from a stored representation, and a render
that turns out safe to share is published for the next matching request. An
application that never opts a route in pays nothing; an opted-in route that
is never actually safe to cache (because its handler reads a session value,
say) still serves correctly, it just never gets stored. Live is complete
without RenderCache, and enabling it changes avoided work, never application
capability.

## Contracts and providers

The engine crate owns the host-neutral contracts, under
`crates/suprnova-live/src/render_cache/`:

- `policy.rs`: `RenderCachePolicy`, its builder, `PolicyPatch`, `QueryPolicy`,
  `FreshnessPolicy`, `CoherenceMode`, `SharedCachePolicy`, `FailurePolicy`,
  `StorageLayers`, and the concrete-response `eligibility` decision.
- `variance.rs`: `VarianceDimension`, `PrivateMaterial`, `DimensionValue`,
  `VarianceDescriptor`, `ObservedContext`, `ClassificationReason`,
  `ClassificationOutcome`, and the `classify` function.
- `key.rs`: the canonical, bounded, versioned `RenderKey`.
- `generation.rs`: `DependencyIdentity`, `GenerationSet`, the
  `GenerationLedger` trait, `ObservationWindow`, and `CoherenceCheck`.
- `entry.rs`: the versioned `CompleteEntry`/`EntryHeader` codec, `SafeHeaders`,
  and body-free `EntryInspection`.
- `store.rs`: the `RenderStore` provider trait and the immutable in-process
  `MemoryRenderStore` (L0).
- `coherence.rs`: `FreshnessState`, `evaluate_freshness`, `ValidationLease`,
  age and warning metadata.
- `http.rs`: conditional evaluation and `Cache-Control`/`Vary` metadata.
- `singleflight.rs`: the `RebuildCoordinator` trait, `LocalRebuildCoordinator`,
  and bounded rebuild admission.
- `lease.rs`: the `LeaseStore` port, its in-memory reference implementation,
  and `FencedLeaseCoordinator`, the kernel that turns a store into
  cross-node rebuild leadership. See Deployment tiers and providers below.

The framework adapts those contracts to Suprnova, under
`framework/src/render_cache/`:

- `mod.rs`: the `RenderCache` facade (`install`, `bump_permission_version`,
  `advance_epoch`, `inspect`, `store_inspection`, `sweep`) and the
  process-installed flag that gates every write-side probe.
- `config.rs`: `RenderCacheConfig::from_env` and its environment variables.
- `collector.rs`: the request-scoped dependency collector, a Tokio task-local,
  and the reserved permission-version identity
  (`permission_version_identity`, a `Config` identity under
  `suprnova.render_cache.permission_version`).
- `middleware.rs`: `RenderCacheMiddleware`, the whole request flow.
- `file_store.rs`: `FileRenderStore`, the file-backed L1 provider.
- `ledger.rs`: `SqlGenerationLedger`, the database-authoritative
  `GenerationLedger` implementation, and its migration presence check.
- `orm.rs`: the write-side hooks that advance generations for supported ORM
  and query-builder writes.
- `migration.rs`: `Migration`, the RenderCache schema migration, and
  `TierMigration`, the four tables the database and Redis providers need on
  top of it.
- `providers/`: the six Tier 1 and Tier 2 adapters (`SqlRenderStore`,
  `SqlLeaseStore`, `SqlInstanceRecordStore`, `RedisRenderStore`,
  `RedisLeaseStore`, `RedisInstanceRecordStore`), the shared dialect and
  Redis plumbing, and the one store-clock helper they all read.
- `registry.rs`: `RenderCachePolicyTable`, `GroupPolicy`, and deterministic
  route/group policy resolution.
- `live.rs`: `LiveDocumentFacts` and the Live-specific decline rule.
- `console.rs`: the two hidden operator commands.
- `telemetry.rs`: the closed counter and attribute names.

Two provider contracts carry the whole cache:

- `RenderStore` (`get`, `publish`, `evict`, `inspect`) is what a layer
  implements. `publish` takes a `PublicationFence` (an epoch and a
  coordinator-minted token; a newer epoch, or an equal epoch with a higher
  token, wins) and a `retention_ms`: the milliseconds after `now_ms` beyond
  which a provider that ages entries off disk may remove the entry,
  regardless of its fence. `u64::MAX` means "never age-swept," which is what
  the in-process L0 store passes, since an epoch bump already makes L0
  entries unreachable and `RenderCache::advance_epoch` clears L0 outright.
  `publish` answers `Published`, `Fenced` (a newer or equal fence already
  holds the key), or `Rejected` (the entry violates a store bound).
- `GenerationLedger` (`current`, `advance`, `epoch`) is database-authoritative
  truth. `current` reads generations by dependency digest, because a decoded
  stored entry carries only digests, never the identity that produced them
  (an identity can spell out an application table name and a record's
  primary key, so keeping identities out of stored bytes and inspection
  output is a privacy property, not an accident). `advance` commits by
  identity, inside the caller's own transaction, so a rolled-back write
  advances nothing.

`RenderCache::install(router, config)` checks that the RenderCache
migration's tables are present, and, for a profile that reaches them, that
the tier migration's tables are too and that every Redis endpoint the
configuration would use answers a `PING`. It builds the Live key ring, then
assembles L0 (`MemoryRenderStore`, bounded by `config.l0`), L1 and the
rebuild coordinator (whichever providers the configured deployment profile
names; under the default embedded profile that is a `FileRenderStore` when
`config.l1` names a directory and otherwise none, with a
`LocalRebuildCoordinator` at a 30 second lease and 128 waiters unless
overridden), the clock, and the SQL generation ledger; it then appends
`RenderCacheMiddleware` to the process-wide global middleware chain and
marks the process installed. Installation never clears the middleware
registry, so an application's own logging, session, CSRF, and auth
middleware are unaffected; it must run **after** every `global_middleware!`
registration that establishes request-scoped locale or identity, since the
middleware reads `Lang::locale()` and `Auth::id()` before the handler runs.
The process-installed flag also gates the write side: `orm::advance` and
`ledger::advance_in_current_transaction` consult it before issuing any SQL,
so an application that never installs RenderCache pays zero RenderCache SQL
on any write. The install is asynchronous because of that first probe, so
`Application::try_routes_async` is the hook that hosts it: the process-wide
and HTTP boot hooks both run before a router exists, and a `booted` callback
is synchronous and never sees one. A configuration whose `enabled` master
switch is false (`RENDER_CACHE_ENABLED=false`) makes the whole call a no-op:
the router comes back untouched, nothing is probed, no runtime is assembled,
no middleware is registered, and the write side's gate stays shut, so an
application that turns the cache off need not carry the migration at all.

`RenderCache::bump_permission_version()` advances a persisted generation on
the reserved permission-version identity, through the same ledger path an
ORM write's advance takes (it joins the caller's ambient transaction when
there is one and opens its own otherwise), and every render whose key
carries a resolved `Principal` value observes that identity, so each such
entry published before the bump fails its coherence check at its next
lookup, in this process and in every later one sharing the database. The
version field the engine's `PrivateMaterial::principal` still accepts is
frozen at 0 by this host: the earlier process-local counter that fed it
reset to 0 on restart while an L1 entry keyed under 0 survived on disk, so
the generation, not the key, is the mechanism. `RenderCache::advance_epoch()`
advances the ledger's authority epoch and clears L0 immediately.
`RenderCache::inspect(key_text)` and `RenderCache::store_inspection()` give
body-free, key-free operator visibility into L0 occupancy and one entry's
metadata. `RenderCache::sweep()` drives L1's bounded disk cleanup; see
Operations below.

## Framework middleware and policy

`RenderCacheMiddleware::handle` runs this flow for every request:

1. Pass through unchanged when no runtime is installed, the request has no
   matched route pattern, the config is disabled, the method is not GET or
   HEAD, or the route has no effective policy.
2. Derive the lookup key from the route identity, path params, the
   declared query names, the declared variance (host, locale, media,
   encoding, tenant, principal), the application build id, and the current
   authority epoch. A query parameter present on the request but not
   declared by the policy bypasses the cache for that request rather than
   silently excluding it from the key.
3. Look up L0, then L1; a decode failure evicts the defective entry from the
   layer it was found in and is treated as a miss there. An L1 hit that
   decodes is promoted into L0.
4. A hit is checked for coherence (see Generations and coherence) and
   resolved to a freshness state, then served: a fresh hit answers 304 when
   the request's `If-None-Match` matches, otherwise the full body for GET or
   headers only for HEAD; a stale-servable hit is served immediately and,
   unless the route's variance depends on task-local context, a bounded
   background rebuild is spawned; a stale-on-error hit triggers a foreground
   rebuild and only falls back to serving the stale entry if that rebuild
   itself fails (a 5xx response or a provider failure); a moved or dead hit
   falls straight through to a render.
5. A miss is admitted through the rebuild coordinator: the leader renders and
   may publish; a waiter reuses the leader's publication once it lands or
   renders without publishing if the leader's cycle failed to publish; a
   request past the waiter cap renders without publishing.
6. The leader's render runs under the request-scoped collector, inside a
   database transaction when a database is configured, opened at
   `REPEATABLE READ` on PostgreSQL and MySQL (`DB::transaction_with_isolation`)
   and at the backend default on SQLite, whose WAL read transaction is
   already a snapshot, so the generations it reads at window-close share one
   snapshot with the data the render itself read. A plain `DB::transaction`
   would be `READ COMMITTED` on PostgreSQL and not a snapshot at all. When
   the key carries a resolved `Principal`, the collector observes the
   reserved permission-version identity before the handler runs.
7. After the render, the response is checked for eligibility
   (`RenderCachePolicy::eligibility`) and classified from what the collector
   actually observed (`classify`); an ineligible or `Uncacheable` response,
   or one an unreasoned-narrowed-private-class check or the Live document
   rules decline, is served without storing.
8. A fresh reread of the observed dependencies and the epoch, taken outside
   the render's own transactional view, catches anything that moved during
   the render; a move discards the candidate.
9. A still-coherent candidate is encoded and published to L0 (and L1 when the
   policy uses it) under a fence minted by the coordinator for this lease.
10. The served response carries `ETag`, `Cache-Control`, `Vary` (from the
    declared variance dimensions that imply one), and `Age`, plus `Warning`
    when the response is stale.
11. A provider failure before the handler ran is decided by the route's
    `FailurePolicy`: `Open` passes the request through uncached, `Closed`
    answers a bare `503`.

`RenderCacheMiddleware` records every attempt as one of eight
`LookupOutcome` values (the `suprnova.render_cache.lookups` counter's
`outcome` attribute):

- `L0Hit` / `L1Hit`: a fresh entry was served from the in-process store or
  the file store (an `L1Hit` is also promoted into L0 before it is served).
- `Conditional`: a fresh hit whose `If-None-Match` matched the entry's
  validator, so the response is a body-free 304.
- `Stale`: a stale-servable entry served immediately, or the stale-on-error
  fallback response served after a foreground rebuild attempt itself failed.
- `Miss`: no entry was found, a stale-on-error hit is attempting its
  foreground rebuild, or a dead entry triggered a fresh render.
- `Bypass`: the request carried an undeclared query parameter, a declared
  variance dimension's value could not be resolved (see "the honest
  boundary" below), or the rebuild coordinator's waiter list was exhausted.
- `Moved`: the fresh reread taken after rendering found that an observed
  dependency, or the epoch itself, had changed since the render's own view;
  the candidate is discarded and never published.
- `Declined`: the rendered response failed eligibility (wrong method or
  status, a streaming body, a `Set-Cookie`, or an unsafe header), the
  collector's report overflowed its observation bound, classification
  landed on `Uncacheable`, the Live document rules declined it, the
  unreasoned-narrowed-private-class invariant fired, the key-versus-render
  value guard (below) fired, a public seed's promotion deadline passed
  between the render starting and publication, or the candidate's headers
  could not be safely encoded.

`suprnova.render_cache.publications` and `suprnova.render_cache.rebuilds`
are plain counts with no `outcome` attribute: a publication is counted only
when the store answers `Published` (not `Fenced` or `Rejected`), and a
rebuild is counted once per spawned background rebuild attempt.

### The honest boundary of what the guards can see

`key_used_different_values_than_the_render_saw` declines to store a render
whose observed locale, principal, or tenant values disagree with what the
derived key actually declared. For every dimension a classification reason
requires, it compares the **entire set** of values the render observed for
that dimension (not just the last one written) against the key's material.
When the render observed nothing for a required dimension, an empty
observed set is safe only when the key itself already says `Private(_)` or
`Anonymous` for that dimension; anything else, including the dimension not
being declared at all, is declined. Locale is checked the same way,
unconditionally, since it is a content-variance concern rather than a
`RepresentationClass` privacy concern.

This guard can only compare what the collector actually recorded. Two
categories of read produce no observation at all, so there is nothing to
compare against, and one category is recorded as unobservable, so the render
is never stored:

- **Headers**, read through `Request::header` and friends, are deliberately
  not instrumented: every request reads some header for some purpose, so
  recording every read would decline every response.
- **Configuration**, read through `Config::get::<T>()`, has no producer: the
  call returns whole typed structs, so a read that touches secret
  configuration is indistinguishable at that seam from one that does not.
- **Raw SQL reads fail closed.** `DB::select`, `DB::select_one`,
  `DB::scalar`, and `DB::select_on` cannot name the tables their statement
  read, so each marks the collector report incomplete
  (`collector::observe_unobservable_read`) and the render is declined rather
  than stored, the same way an overflowed report is; the response is still
  served, and the framework's own RBAC role and permission checks
  (`rbac::has_roles`) read this way, so a cached route that evaluates one
  never stores. The query-builder facade is not in this category:
  `DB::table(..).get()`, `first()`, and `count()` know their table and record
  it (`collector::observe_table_read`), which is also how `Auth::user()`
  resolving through `DatabaseUserProvider` observes the `users` table with
  no change of its own. Writes are classified by a prefix heuristic
  (`is_select_statement`): a raw statement that does not begin with `SELECT`
  is treated as a write and advances `Broad`, the safe direction, while a
  side-effecting `SELECT nextval(..)` is treated as a read and advances
  nothing.
- **An Eloquent global scope's own per-request state.** `ScopeRegistry`'s own
  documentation invites a `GlobalScope::apply` implementation to read
  per-request state, such as the current tenant, from an application-defined
  thread-local, `tokio::task_local!`, or atomic, and filter the query by it.
  A query built through `Model::query()` this way changes the render's body
  with nothing here ever observing the read. A route whose models carry a
  tenant-scoped global scope needs its own declared `Tenant` variance;
  nothing here can detect the omission.

Two narrower cases are exceptions, not full coverage:

- **Cookies** (`Request::cookies`/`Request::cookie`) are instrumented as a
  session read, since cookies carry private material by nature.
- **Feature flags** are observed only through the two evaluators this
  framework ships (`DatabaseEvaluator`, `CachedEvaluator`), only through
  identity, and only on the axis a flag actually has a scoped rule for: a
  `user:`-scoped rule records the reader's user id as principal material (or
  a bare principal read when the reader carries none), and a `team:`-scoped
  rule records the team as tenant material (or a bare tenant read) the same
  way. A flag with only a global rule records nothing on either axis. A
  custom `Evaluator` outside these two, or a scope key that is neither
  `user:` nor `team:`, is invisible.

Two further limits are documented, deliberate gaps rather than guard
weaknesses:

- **`Auth::id()`'s session fallback stays a session read.** `Auth::id()`
  resolves through request state first and falls back to `session()` for an
  anonymous visitor; `session()` always records a session read, and any
  session read narrows straight to `Uncacheable` inside `classify`. So an
  anonymous visitor of a route whose render calls `Auth::id()` never caches,
  even though the key correctly resolves to `Anonymous` for that visitor. A
  signed-in visitor resolves through request state and never reaches the
  fallback, so the same route does cache for them.
- **Authorization decisions are always treated as per-principal.**
  `Gate::allows` records only that a decision was evaluated, never what it
  consulted, so an `AuthorizationRead` reason always requires the
  `Principal` dimension. A route keyed only by `Tenant` whose gate check is
  genuinely per-tenant never caches unless it also declares `Principal`.
- **Every Inertia document render observes a locale.** Inertia's own
  document shell builds `<html lang>` from `Lang::locale()` unconditionally,
  so this observation happens whether or not the page's own data has
  anything to do with language. The value guard therefore declines any
  Inertia route that does not declare `Locale` variance, on every request:
  this fails closed rather than leaking, but it fails silently from the
  response's own point of view, and declaring `Locale` is the fix.
- **Only the serving process advances generations.** The write-side
  instrumentation is opened by `RenderCache::install`, which only the
  serving process calls (`serve` and `web-run`), so writes from queue
  workers, scheduled tasks, and console commands advance no generation and
  `bump_permission_version` there is a no-op. A page that depends on such a
  write stays current only within its freshness window; `advance_epoch` is
  the operator remedy after a job that changes cached content.

A route handler that branches its output on a header or a config value,
without also declaring the matching variance, is outside what this
middleware can protect on its own.

## Privacy classification

`RepresentationClass` has four variants, ordered from widest to narrowest
sharing: `PublicShared`, `PublicShellStitched`, `PrivateCached`,
`Uncacheable`. `RepresentationClass::narrowest` is `self.max(other)`, so
classification can only ever move a route's declared class toward
`Uncacheable`, never back toward wider sharing.

`classify(declared, observed)` starts from the route's declared class and
narrows on six `ClassificationReason` variants, each attached to what it
observed:

- `PrincipalObserved` (a signed-in principal was observed) narrows to
  `PrivateCached`.
- `TenantObserved` (a tenant was observed) narrows to `PrivateCached`.
- `AuthorizationRead` (a private authorization decision was evaluated)
  narrows to `PrivateCached`.
- `SessionValueRead` (a session value was read) narrows to `Uncacheable`.
- `SecretContextRead` (secret configuration or feature context was read)
  narrows to `Uncacheable`.
- `UndeclaredContext` (request context outside the declared variance
  affected rendering) narrows to `Uncacheable`.

`ClassificationOutcome` carries the resulting class plus every reason that
fired, in evaluation order, which is what the value guard described above
walks to decide whether the key actually reflects what was observed.

Separately, `RenderCachePolicy::eligibility` narrows a route's declared
class to `PrivateCached` whenever the concrete response's own signals show
private material was observed, on top of declining outright for a
non-GET/HEAD method, a non-200 status, a streaming body, a response that
sets a cookie, or a response carrying a hop-by-hop, connection, or tracing
header (`UNSAFE_RESPONSE_HEADERS`).

The documented limits from "the honest boundary" apply directly to
classification: a session fallback keeps anonymous identity-touching
renders `Uncacheable` rather than `PrivateCached`; per-tenant authorization
requires a route to also declare `Principal`; header, `Config::get`, and an
Eloquent global scope's own task-local reads are invisible to
classification entirely; a custom feature-flag evaluator, or a scope key
that is neither `user:` nor `team:`, is invisible too; and an Inertia
document's unconditional locale read means the route must declare `Locale`
or it is declined on every request.

## Generations and coherence

`DependencyIdentity` is a closed, typed dependency: `Table`, `Record`
(table plus primary-key bytes), `QueryClass` (a named query class over a
table), `Relation` (a parent/child table pair), `Config`, `Feature`,
`Locale`, `Route`, and `Broad` (the always-observed authority every
representation depends on; an epoch change is reported as `Broad`'s own
digest). `GenerationLedger::current` reads generations by 32-byte dependency
digest, since a decoded stored entry carries only digests, never the
identity that produced them; `GenerationLedger::advance` commits by
identity, since the write path that calls it always knows the identity it
just changed. `MAX_OBSERVATIONS` bounds one representation's dependency set
at 4,096; the framework's own collector reserves one slot below that bound
so a report that is otherwise full can still fold in the always-present
`Broad` seed and close successfully.

`ObservationWindow::open(epoch)` seeds the window with `Broad` and the
opening epoch; `observe` records one identity, bounded and idempotent under
the same `MAX_OBSERVATIONS` limit; `close(ledger)` reads every observed
identity's current generation from the ledger, by digest, and returns a
`GenerationSet`.

Two separate reads make up coherence around one render:

- **The consistent read view.** The leader's render runs inside a
  transaction opened through `DB::transaction_with_isolation` when a
  database is configured, at `REPEATABLE READ` on PostgreSQL (whose default
  `READ COMMITTED` gives every statement the latest committed data, not a
  snapshot) and on MySQL (where InnoDB already defaults to it), and at the
  backend default on SQLite (a WAL read transaction is a snapshot as of its
  first read; the pinned SeaORM would only log a warning for a level there).
  The observation window closes (reading the ledger) while that transaction
  is still open, so the generations it reads share one snapshot with
  whatever data the render itself read: a write that commits mid-render is
  invisible to both, the candidate carries the pre-write generations, and the
  fresh reread below discards it. The consequence for authors: on PostgreSQL
  a cached route's handler that updates a row another transaction changed
  after the render began sees a serialization failure; cached routes are
  read paths. Proven against a live PostgreSQL in
  `live_postgres_a_write_committed_during_a_cached_render_is_never_published_as_current`
  (and the MySQL twin), which `scripts/check-postgres.sh` and
  `scripts/check-mysql.sh` run and assert on by name. Those two scripts each
  carry a second, later block that runs the Tier 1 adapter regressions by
  name on the same servers, and `scripts/check-redis.sh` is their Tier 2
  counterpart; see Deployment tiers and providers below.
- **The owning transaction.** An ORM write inside a `DB::transaction`
  advances its generations inside that same transaction; a bare autocommit
  write (`model.save()` with no ambient transaction) has already committed
  its row when the advance opens an immediately following dedicated
  transaction, so the window between the two is "new data, old generation",
  which costs one extra rebuild and never serves stale content.
- **The fresh reread.** After the render finishes and classification and
  the value guard both pass, `fresh_reread_is_coherent` rereads the observed
  dependencies and the epoch again, outside the transactional view this
  time. `CoherenceCheck::compare` reports `Coherent` or `Moved(digests)`
  (an epoch change is reported as `Broad`'s digest); any move discards the
  candidate rather than publishing it.

On a hit, `coherence` decides currentness before freshness is even
evaluated: `CoherenceMode::Authority` rereads the ledger on every hit;
`CoherenceMode::Lease { max_age_ms }` trusts a locally granted, still-valid
`ValidationLease` instead, rereading (and granting a fresh lease on a
coherent result) only once the lease has expired. A lease's own hint can
only shorten its expiry, never extend it. Lease mode does not need its own
epoch comparison: `RenderKey::derive` bakes the current epoch into the
lookup key itself, so an epoch bump changes the key for every route,
lease-mode routes included, making a previously published entry unreachable
by ordinary lookup on the very next request rather than something a hit
path would ever need to detect as "moved."

`evaluate_freshness` resolves one of four states from a policy's
`FreshnessPolicy` (`fresh_ms`, `stale_servable_ms`, `stale_on_error_ms`):
`Fresh`, `StaleServable`, `StaleOnError`, or `Dead`. A stored public-seed
deadline at or before the current time is `Dead` regardless of every other
interval, since a seed past its promotion deadline can never be promoted
again. A `PrivateCached` representation is never served stale: past its
fresh interval it goes straight to `Dead`. The engine derives the Dead edge
itself per representation class rather than from one class-blind formula,
so a private entry's edge lands at the end of its fresh interval while a
public entry's edge accounts for both stale windows beyond it; this is the
single source of truth both `evaluate_freshness` and L1's own
retention-based cleanup use, so a live freshness check and a disk sweep can
never disagree about when an entry is truly dead.

## Live documents

`LiveDocumentFacts` accumulates across every island mount and every
rendered document in one request: `public_seed_islands` and
`identity_bound_islands` counts, the earliest `seed_deadline_ms` across
every mounted public-seed island, a sticky `no_store` flag, and the
`StitchCapture` a stitched shell would be cut from (see Composite stitching
below). Mount facts are recorded from `LiveDocument::mount` itself,
immediately after a mount succeeds, rather than from `render` - a handler
can mount an island and hand-build its own response from
`MountedIsland::html()` without ever calling `render`, so recording at mount
means the fact exists regardless of whether `render` is reached. The stitch
capture is recorded there too, and on every route rather than only a
stitched one: `LiveDocument::mount` records a public-seed island as staying
inside the shell and an identity-bound island together with its own emitted
markup, with no class check at either site, and the collector keeps whatever
it is handed regardless of what the route declared. Only the consumption is
class-gated - `document_declines` reads `stitch.invalid` for the stitched
class alone, and the composite publisher runs for that class alone - so
every other class ignores what was recorded. That is also why
`CapturedSlot`'s `Debug` is hand-written to print lengths rather than
markup: an island's bytes and its signed snapshot sit in the capture of
every Live request, and the capture is reachable from a public derived
`Debug`. A rendered document's cache intent is recorded separately, and
only when it is `NoStore`: `Private` and `Public` intents
neither narrow nor widen this server-side cache's class, since
`DocumentResponseIntent::html()` defaults to `Private` and mapping that
default to `RepresentationClass::PrivateCached` would demote every Live
document with no `ClassificationReason` behind the demotion for the value
guard to check the key against. The route's own `RenderCachePolicy`, not
the document's intent, governs this server-side cache; the intent governs
only the downstream `Cache-Control` a browser or CDN sees.

`document_declines(facts, declared)` can only decline a render, never narrow
or widen what `classify` already decided: a document that declared
`NoStore`, a public-seed island whose promotion deadline could not be
resolved, an identity-bound island mounted on a route that did not declare
`RepresentationClass::PublicShellStitched`, or a capture the stitched
publisher cannot trust, all decline storage outright. The route's
**declared** class is what it reads, not the class `classify` produced,
because an identity-bound island is exactly what a stitched route exists to
re-render on every hit: declaring that class is what turns the island from a
reason to decline into the reason to stitch. A seed's remaining time is also
checked once more immediately before publication (`seed_remaining_ms`); if
the deadline is reached between the render starting and this point, the
candidate is declined rather than stored already dead.

## Composite stitching

`RepresentationClass::PublicShellStitched` is the class for a document whose
shared parts are the same for every visitor and whose islands are not. A
render under it is stored as an `EntryKind::Composite` entry: a shared
**shell** of bytes with typed holes cut in it, plus a **segment graph**
saying what goes back into each hole. No identity-bound island's markup and
no signed snapshot is ever inside the stored bytes. Every hit re-mounts
every island for whoever is asking, under authority derived for that request
alone, so a stitched route trades one shared render of the page frame for
per-request rendering of exactly the parts that depend on identity.

The engine owns the entry form and the assembler
(`crates/suprnova-live/src/render_cache/composite.rs`, `entry.rs`); the
framework owns capture, publication, and the hit path
(`framework/src/render_cache/stitch.rs`, `collector.rs`, `live.rs`,
`middleware.rs`, and `framework/src/live/document.rs`).

### The segment graph and its bounds

A `SegmentGraph` is an ordered list of segments over the shell: `Literal`
(the next `len` bytes of the shell), `Slot` (the output of `slots[index]`),
and `Nonce` (the nonce minted for this assembly). The literal segments
always sum to exactly the shell's length, so the shell is partitioned rather
than searched. Each `StitchSlot` records what a later hit needs to mount its
island again - route identity, island slot, document mount key, component
name, contract digest, protocol version, build id, RFC 8785 canonical mount
parameters, inert mount flags - plus the declared failure policy and a
`surrounding` digest over the shell bytes on either side of the slot. Public
seeds are the same for everybody, so they stay inside the shell and the
graph records only that they are there (`shell_islands`), which is how a hit
knows their document mount keys are already taken.

| Bound | Value |
|---|---|
| `MAX_STITCH_SLOTS` | 32 |
| `MAX_NONCE_HOLES` | 64 |
| `MAX_SEGMENTS` | 193 (`2 * (32 + 64) + 1`) |
| `MAX_SLOT_PARAMETER_BYTES` | 4,096 |
| `MAX_FALLBACK_BYTES` | 4,096 |
| `MAX_SHELL_ISLANDS` | 128 |
| `MAX_NONCE_HEADERS` | 4 |
| `SURROUNDING_WINDOW_BYTES` | 64 |
| `MAX_NONCE_BYTES` | 256 |

`CompositeEntry::new` validates every structural rule and bound against the
shell's length and binds the canonical header, the graph, and the shell into
a `structural_digest`. That digest is not an HTTP validator, and a Composite
entry has none: the bytes it stands for do not exist until a request
assembles them.

### Capture at mount time

The facts a shell is cut from are recorded as the document is built, not
afterwards. `LiveDocument::mount` records each identity-bound island's
`StitchSlotDescriptor` together with the exact `TrustedHtml` bytes that mount
produced (`record_stitch_slot`), records each public-seed island as staying
inside the shell (`record_shell_island`), and marks the capture unusable
when a mount's canonical parameters exceed the slot bound
(`record_stitch_capture_invalid`) rather than turning a working document
into an error. `LiveDocument::bootstrap` records the Content Security Policy
nonce its markup stamped, and `LiveDocument::render` records a SHA-256 of
the body it produced. All of it lands in `StitchCapture` inside
`LiveDocumentFacts`, in mount order, which is also document order.
`CapturedSlot` and `SlotFailurePolicy` both have hand-written `Debug`
implementations that print lengths rather than markup, because the capture
is reachable from a public derived `Debug` and an island's markup carries
that principal's signed snapshot.

An application declares a slot's failure behaviour on the mount itself, with
`LiveMount::on_stitch_failure`. The default is
`StitchFailurePolicy::FailDocument`; `Omit` and `Fallback(TrustedHtml)` are
the alternatives, and a fallback fragment larger than `MAX_FALLBACK_BYTES`
is rejected where it is written rather than silently at publication. On any
other class the declaration is accepted and inert.

### Attribution: gate, content, slot

The request-scoped collector puts every read into one of three buckets. A
scope starts in the **gate** bucket, which holds whatever ran before the
route handler - an authorization guard, tenant middleware. `begin_handler`,
called by the Live completion middleware, switches to the **content**
bucket, which holds what the handler read to build the body. `slot_scope`
runs an identity-bound island's mount in the **slot** bucket, whose reads
are counted and recorded nowhere else.

Only `PublicShellStitched` classifies from the content bucket alone. Three
things together are what earn that exemption. First, the stored shell holds
only what the handler rendered after `begin_handler`, so a gate's own bytes
are never in it, and a gate that rewrites the body after the handler
returned is caught by the body digest, which declines the store rather than
publishing the rewritten bytes. Second, every hit on this class runs the
route's own gate again before a byte is served, so the gate's decision is
taken fresh per request instead of being read back out of the shell. Third,
a gate value that also reaches the handler through an instrumented seam
(`Auth::user()`, a cookie, a query-builder read) is observed a second time
in the content bucket when the handler consumes it, so it still reaches the
key. What is left is a gate value reaching the handler through an
uninstrumented seam - a request header, an application task-local - and that
is the pre-existing boundary described under "The honest boundary of what
the guards can see" above, which the gate bucket never closed and this
exemption does not widen. Every other class folds the gate bucket back
into content (`CollectorReport::fold_gate_into_content`) and classifies from
exactly the undivided report it produced before attribution existed. A
stitched route whose chain answered before the handler ever ran has an empty
content bucket, and classifying from it would publish the gate's own
response as the shared shell; the report records that in
`CollectorReport::handler_began` and the middleware declines to store the
representation when it is false.

### The six checks before publication

Six checks stand between a stitched render and a stored shell. The first is
the middleware's own, enforced in two different places; the rest are
`stitch::build_composite_entry`'s, the only publisher for a stitched route
that rendered a Live document. Every rejection is a decline counted under
the existing `declined` lookup outcome, never an error:

1. The handler began, and the Live document rules do not decline outright.
   The handler-start half is enforced earliest, in `render_under_collector`:
   a stitched report with no handler start is given no observed generation
   set at all, which reuses the "no generation set" signal an overflowed
   report already returns rather than adding a telemetry label. The Live
   document half is `document_declines(facts, declared)`, which runs
   immediately after classification and takes the route's **declared**
   class, because an identity-bound island is exactly what this class exists
   to re-render on every hit: under `PublicShellStitched` that island is the
   reason to stitch, and under every other class it is still a reason to
   decline. A capture marked invalid, a document that declared `NoStore`, or
   a public-seed island with no resolvable deadline all decline there.
2. The capture accounts for every identity-bound island the request mounted
   (one captured slot each) and holds no more than `MAX_STITCH_SLOTS`.
3. The response body is byte for byte the body `LiveDocument::render`
   produced. Route middleware that rewrites the body afterwards runs again
   on every hit, so storing its output would apply it twice and leave the
   entry's validator describing bytes no client ever received. This check is
   also what bounds everything below: the body is now provably the rendered
   document, which the view renderer already bounded.
4. Every captured island's bytes occur exactly once in that body, and the
   occurrences do not overlap. An island found twice, or not at all, cannot
   be cut out, and a shell that kept it would be that principal's markup and
   signed snapshot, shared.
5. Every occurrence of the document's bootstrap nonce outside every island
   is collected as a hole. The scan is bounded at `MAX_NONCE_HOLES + 1`
   matches and declines the whole document the moment it reaches that limit,
   before any filtering: a truncated scan cannot prove there is no further
   occurrence, and one it did not see would be copied into the shared shell
   as a fixed nonce while every hit rebuilt the header with a fresh one.
6. Every replayable stored header whose value carries that nonce becomes a
   `HeaderTemplate` of text and nonce pieces, within `MAX_NONCE_HEADERS`
   templates and `MAX_NONCE_HOLES` pieces each.

Only then is the form decided, and it can be decided because checks 4 to 6
between them enumerated everything that has to come out. A document with no
islands, no nonce holes, and no nonce-bearing header is a finished shared
answer and is published `Complete`. Anything else is published `Composite`,
including a document with no islands but a nonce: a `Complete` entry there
would freeze the first visitor's nonce into both the stored body and the
stored `Content-Security-Policy` and replay them to everybody, which is a
nonce that proves nothing. The body is then walked once, cutting at every
island and every hole; what is not cut out becomes the shell, and each
slot's surrounding digest is taken over the shell that resulted.

### The hit path

The global `RenderCacheMiddleware` never answers a stitched route where it
stands. On a hit it decodes the entry, fixes the freshness decision and the
instant it was taken at, attaches the result to the request
(`Request::attach_prepared_hit`) and calls the next layer, so the route's
own authorization guard, tenant middleware, and anything else it declared
run exactly as they do on a miss. The Live completion middleware - the last
middleware before the handler - is what finally serves the hit, through
`stitch::serve_prepared`. A request the chain refuses first never reaches
that point at all, and the prepared hit is dropped unread with the request.
A `Complete` entry prepared this way replays through the same conditional
response a non-stitched hit would use; a `Composite` entry is assembled.

Assembly re-derives every slot from the live mount catalog rather than
trusting the entry. `render_slot` looks the registration up by route and
island slot and accepts it only when it is still identity-bound and its
component, contract digest, protocol, document mount key, and build all
match what was stored; anything else is drift between the stored entry and
the running build, and drift is a slot failure, never a substitution. The
request context is then validated through the same path a handler's own
mount would use, and the island is mounted inside `slot_scope`.
Reauthorization is per request and is never cached. Before any slot is
mounted, the document mount scope reserves the shell's public-seed keys, so
a stitched slot can never re-mount under a key the assembled document
already contains.

Failure is per slot and follows that slot's declared policy: `Omit` and
`Fallback` are recorded as such and assembly continues, `FailDocument`
abandons the document. Anything that is not a slot's business - no runtime,
an unparseable path or stored declaration, a shell whose islands cannot be
reserved, an exhausted randomness source, an engine assembly that rejects
the result - fails the document as a whole rather than serving a partial
one. Every one of those paths ends at `fail_document`, which counts the
outcome and lets the route's own handler answer, uncached, exactly as it
would on a miss. There is no partial response.

`assemble` itself is pure and deterministic. It checks that each outcome
names the slot it was rendered for and obeys that slot's declared policy,
recomputes every surrounding digest from the graph and the entry's own shell
(so a shell that drifted after the digest was recorded is caught), computes
the exact final length from typed facts alone and enforces the body bound
before allocating a byte, walks the segments once, and rebuilds the
replayable headers from the stored header plus every nonce template.

### The assembled response

The served headers come from the assembled document, not from the stored
header, because a nonce-bearing header has been rebuilt around the nonce
minted for this request; replaying the stored value would declare the
leader's nonce over a body carrying somebody else's. `Vary`, `Age`, and
`Warning` follow the shared contract. Two things differ from every other
class, and both follow from the bytes being new:

- **A Composite response never answers 304.** `If-None-Match` is not
  evaluated at all. Every assembly is a distinct representation - a fresh
  nonce, fresh instance identities - so a 304 would tell the client to pair
  the body it already has with headers minted for this request. The
  validator is still strong over exactly the bytes sent and still honest
  about which representation this is; it simply never matches a later
  request, which is the truth. `HEAD` still sends the headers with no body.
- **A slotted assembly is `private, no-store`.** The bytes hold islands
  mounted for one principal under authority re-derived for one request; a
  `max-age` would let a shared browser profile replay them to whoever sits
  down next and skip reauthorization for the whole window. A zero-slot
  Composite has no per-principal bytes in it, only a per-request nonce, so
  it keeps the class's private `max-age` like any other private
  representation.

The class refuses `SharedCachePolicy::SMaxAge` at policy build time, so no
shared proxy is ever told to keep bytes the server never cached.

### Telemetry and test seams

Two counters, both with a closed `outcome` attribute:
`suprnova.render_cache.stitch.assemblies` (`assembled`, `fail_document`) and
`suprnova.render_cache.stitch.slots` (`rendered`, `omitted`, `fallback`,
`failed`). They are declared in `framework/src/render_cache/telemetry.rs`
alongside the four lookup, publication, and rebuild names, and are listed
with them under Operations below.

Two hidden test seams reach state no external test could otherwise produce.
`RenderCache::shell_for_test` returns a stored Composite entry's shell bytes,
which is what proves the shell holds no island markup and no signed
snapshot; `EntryInspection` reports only a shell's length.
`render_cache::testing::rewrite_composite_for_test` rewrites a stored graph
in place and republishes it under the same key with a fresh fence, which is
how a test reaches a redeploy: a stored slot naming a component or contract
digest the running registry no longer has. Both are `#[doc(hidden)]` and
gated on `cfg(test)` or the `testing` feature.

The application dogfoods the class on its own dashboard: `app/src/live/mod.rs`
declares `PublicShellStitched` for `/live`, whose three islands are
identity-bound and whose shell reads nothing private, and
`app/tests/live_render_cache.rs` proves through the running application that
one shared shell is stored as a Composite entry with three slots, that a
second principal is served from it without the handler running, that each
principal's island carries its own scope, that the response is
`private, no-store`, that a conditional GET is answered 200, and that an
anonymous visitor gets the route's own login redirect.

### Limitations

Each of these is ruled behaviour, not a defect.

- `PublicShellStitched` is meaningful only on routes whose chain ends in the
  Live completion middleware. A non-Live route under the class never
  short-circuits on a hit: the prepared hit is dropped unconsumed and the
  handler re-renders, and the route still publishes an entry it can never
  serve. This is unconditional on purpose, because answering such a hit in
  the global middleware is the exact short circuit the class exists to
  forbid.
- A stitched route whose handler renders no Live document at all publishes a
  `Complete` entry without the body-digest rule, so response-rewriting route
  middleware on such a route applies again on every hit. Use the class only
  with `LiveDocument::render`.
- Response-rewriting route middleware on a stitched Live route makes the
  document decline at publication (body digest mismatch), so the route is
  served uncached on every request.
- A document with more than 64 occurrences of its bootstrap nonce in the
  rendered body (islands included, since the scan declines before it filters
  them out), more than 32 identity-bound islands, or more than 193 graph
  segments declines under the generic `declined` outcome; there is no
  dedicated telemetry reason for a bound.
- A replayable header that carries the nonce is budgeted against the
  4,096-byte header value bound at `MAX_NONCE_BYTES` (256) for each nonce
  piece, not at the length of the nonce this document actually stamped,
  because the nonce that lands there is minted per hit and is not known when
  the entry is validated. A `content-security-policy` value close to 4,096
  bytes therefore declines even when the document carries a single short
  nonce.
- An assembled document with at least one private island is sent
  `Cache-Control: private, no-store`; a zero-island Composite keeps the
  class's private `max-age`.
- Composite responses never answer 304, so `If-None-Match` is ignored and the
  emitted `ETag` serves `HEAD` and same-response validation only.
- A stitched hit whose route chain refuses it (authorization, tenant) has
  already been counted as a hit before the chain ran; the stitch assembly
  and slot counters are the ones that describe what assembly actually did.
- A slot failure on a hit is not distinguished in telemetry by cause: a
  missing registration, an identity mismatch, an authorization refusal, and
  a mount error all count as `failed` or follow the slot's declared policy.
  A follow-up capture may add reasons, as
  `iterations/next/declined-lookups-record-a-reason.md` proposes for the
  lookup outcome.
- A stitched entry is never served by the stale-on-error fallback and never
  triggers a background rebuild. Both gates key on the route's policy class,
  so a zero-slot Composite is excluded from each as much as a slotted one
  is. Serving the stored shell on a failed foreground rebuild would answer a
  request the route's own chain never got to gate, and a spawned background
  rebuild carries none of the request's authorization task-locals, so its
  shell would be whatever the gate renders for nobody. A stale-servable
  entry is still served immediately, assembled where it is a Composite one,
  with the `Warning` header.
- An unencodable read inside a private island's mount marks the whole
  collector report overflowed, so the shell is not stored. This is
  conservative: the read belongs to an island that is re-rendered on every
  hit, but the report cannot say so.
- A Composite entry found under a route whose class declaration changed is
  not evicted. `deliver_hit` counts it as a miss and runs the chain, so
  nothing composite is ever served, but the entry stays in L0 until eviction
  pressure, an epoch advance, or a republish removes it.

## Deployment tiers and providers

Tier 0 is one process: an in-process L0, an optional file L1, an in-process
rebuild coordinator, and an in-process Live instance ledger, over an
application database that already holds generation truth. Tier 1 and Tier 2
keep every one of those semantics and move the three cross-node ones into a
store several processes share. A **profile** names which providers a process
builds; route declarations, policies, the collector, the key, the codec, the
middleware flow, and composite stitching are the same at every tier, and no
application-facing type changes between them.

| Tier | Profile | L1 entries | Rebuild leadership | Live instance records |
|---|---|---|---|---|
| 0, Embedded | `embedded` | one file per key, or none | in process | in process |
| 1, Database-coordinated | `database` | `suprnova_render_entries` | `suprnova_render_leases` | `suprnova_live_instances` and `suprnova_live_promotions` |
| 2, Externally accelerated | `redis` | one Redis hash per key | one Redis hash per key | one Redis hash per record |

Generation truth does not move. `SqlGenerationLedger` is the
`GenerationLedger` at all three tiers, so the coherence check that runs on
every hit is a database read whatever served the bytes. That is what keeps
an accelerator an accelerator: Redis can lose everything it holds without
anything stale being proven current, because nothing Redis holds proves
currency in the first place.

### Profiles and configuration

`RenderCacheConfig` carries a `Profile` (`Embedded`, `Database`, `Redis`), an
`L1Config` (`Disabled`, `File`, `Database`, `Redis`), and a
`CoordinatorConfig` (`Local`, `Database`, `Redis`). The profile sets the
other two; each can then be overridden on its own, so a deployment that wants
its entries in the database but its rebuild leases in process says exactly
that rather than choosing the nearest whole profile.

| Variable | Default | Meaning |
|---|---|---|
| `RENDER_CACHE_PROFILE` | `embedded` | `embedded`, `database`, or `redis`; sets the two below |
| `RENDER_CACHE_L1` | the profile's | `disabled`, `file`, `database`, or `redis` |
| `RENDER_CACHE_COORDINATOR` | the profile's | `local`, `database`, or `redis` |
| `RENDER_CACHE_L1_BYTES` | 1 GiB | the whole directory for the file tier, one entry for the database and Redis tiers |
| `RENDER_CACHE_REDIS_URL` | `REDIS_URL`, then `redis://127.0.0.1:6379` | where both Redis cache tiers connect |
| `RENDER_CACHE_REDIS_PREFIX` | `suprnova_render:` | the key namespace both Redis cache tiers write under |
| `RENDER_CACHE_LEASE_MS` | 30,000 | rebuild lease lifetime |
| `RENDER_CACHE_MAX_WAITERS` | 128 | in-process waiter ceiling |
| `LIVE_LEDGER_DRIVER` | `memory` | `memory`, `database`, or `redis` |
| `LIVE_REDIS_URL` | `REDIS_URL`, then `redis://127.0.0.1:6379` | where the Redis ledger driver connects |
| `LIVE_REDIS_PREFIX` | `suprnova_live:` | the key namespace the Redis ledger driver writes under |

`RENDER_CACHE_L1_DIR` keeps exactly the meaning it had: under the embedded
profile, setting it is still what turns L1 on at all. A variable with a
closed set of accepted values that is set to something outside it fails the
boot with a message that names the variable and never repeats the rejected
value.

The Live instance ledger is configured separately from the cache, because it
is Live's authority rather than the cache's storage. `LIVE_LEDGER_DRIVER`
chooses where instance records live, and a deployment can run the cache on
one tier and the ledger on another.

Both installs fail closed. `RenderCache::install` refuses a configuration
that reaches a tier table the migration has not created, and pings every
distinct Redis endpoint the configuration would use, once per endpoint. The
Live driver is probed by `live::verify_ledger_backend`, which `Server::run`
calls before any request is served: `LiveRuntime::bind` is synchronous and is
reached from synchronous public constructors, so the probe cannot live inside
it.

### The two engine kernels

Neither distributed provider could be written in the framework: the engine's
lease and ledger types keep their constructors crate-private, which is what
stops a host from minting authority the engine did not issue. So the engine
gained two host-neutral kernels over two small store ports. A kernel owns
every semantic; a port owns only atomicity and time.

`render_cache::lease::FencedLeaseCoordinator<S: LeaseStore>` implements
`RebuildCoordinator` by composing a `LocalRebuildCoordinator` (in-process
waiters, unchanged) with a store that decides leadership across processes.
`LeaseStore` is three operations, `try_acquire`, `mint_token`, and `release`,
and `LeaseAttempt` is `Acquired { lease_id, expires_at_ms }` or `Held`.
Admission runs the local coordinator first; `Wait` and `Bypass` pass through,
and a local `Lead` then asks the store. `Acquired` is a `Lead` carrying the
distributed lease id. `Held` answers `Bypass`, after handing the local lease
back so this node's own waiters wake and re-admit rather than parking behind
a leader that will never publish. `publish_token` mints from the store and
turns a `None` into `RenderCacheErrorKind::LeaseFenced`, on which the
middleware discards the render's bytes and publishes nothing. Tokens are
monotonic per key across tenures, so a token minted under an older lease can
never outrank one minted under a newer one.

`ledger::distributed::DistributedInstanceLedger<S: InstanceRecordStore>`
implements `LiveInstanceLedger` by loading a record, applying one of the pure
transitions in `ledger::state`, and compare-and-storing the result at exactly
the version it read. A `Conflict` retries once from a fresh read and then
reports `LedgerErrorKind::Contention`, a classified rejection rather than a
partial state. `MemoryInstanceLedger` keeps its public name and constructor
and is now this kernel over an in-memory store, so Tier 0 and both
distributed tiers run one state machine and answer one conformance suite.

`ledger/record.rs` is the record's only encoding: a `RECORD_VERSION` byte
followed by the RFC 8785 canonical JSON of a mirror of the in-memory record,
the whole frame bounded at `MAX_RECORD_BYTES` (32,768). Every identity
travels as text and comes back through its own validating constructor, so a
record read from a store another process can write is validated exactly as
protocol input is: decoding classifies and never panics, whatever the bytes
are. Records carry revision metadata only, never component state, rendered
HTML, or action arguments. `LedgerError::new` is public, as
`RenderCacheError::new` already was, so a host adapter can report
`ProviderUnavailable` or `CapacityExceeded` without engine help.

### The six adapters and the four tables

The adapters live in `framework/src/render_cache/providers/`. Each carries a
decision to a backend and back; none of them makes one.

| Adapter | Implements | Storage |
|---|---|---|
| `SqlRenderStore` | `RenderStore` | one row in `suprnova_render_entries` |
| `SqlLeaseStore` | `LeaseStore` | one row in `suprnova_render_leases` |
| `SqlInstanceRecordStore` | `InstanceRecordStore` | one row in `suprnova_live_instances` or `suprnova_live_promotions` |
| `RedisRenderStore` | `RenderStore` | a `<prefix>entry:<key>` hash |
| `RedisLeaseStore` | `LeaseStore` | a `<prefix>lease:<key>` hash plus a `<prefix>token:<key>` counter |
| `RedisInstanceRecordStore` | `InstanceRecordStore` | `<prefix>instance:` and `<prefix>promotion:` hashes, indexed by a `<prefix>instances` sorted set |

`TierMigration` (`m20260906_000000_create_render_cache_tier_tables`) creates
the four tables, and a non-unique index on `expires_at_ms` for the three that
are reclaimed, so a bounded sweep never scans the table; leases are taken
over by primary key and need none. Render keys are stored as
`RenderKey::to_base64url()` (`rk1.` plus 43 base64url characters), the lookup
key itself and never a second hash of it. `scope` is `CHAR(64)`, the hex of a
fixed 32-byte fingerprint; `instance` and `idempotency` are `VARCHAR(64)`,
because `InstanceId` and `IdempotencyKey` are 16 to 32 bytes carried as hex.

`L1Provider` is an enum (`File`, `Database`, `Redis`) rather than a trait
object, for one reason: reclamation is provider-specific and is not part of
the `RenderStore` contract, so `RenderCache::sweep` has to reach the
provider's own sweep. Every read and publication goes through the enum's own
`RenderStore` implementation, which delegates and nothing more.

Publication is fenced inside the store, not around it.
`SqlRenderStore::publish` reads the stored `(epoch, token)` under a row lock
where the dialect has one, carries `PublicationFence::supersedes` into the
upsert's own guard as well (a lock cannot hold a row that does not exist yet,
and two nodes publishing a brand-new key can both find it absent), and
re-reads in the same transaction, so the outcome is a stored fact rather than
a dialect-specific affected-row count. `RedisRenderStore::publish` makes the
same comparison inside one Lua script that returns an explicit status. Every
SQL statement binds its values and every script receives `KEYS` and `ARGV`;
no caller value is ever spliced into SQL or Lua text, and no error message
carries a key, a byte, a record, or a URL.

### Store time, and the offset seam

Every cross-node expiry decision is made on the backend's own clock, read
inside the operation that acts on it. `sql_now_ms(backend)` is the one place
the dialects disagree:

| Backend | Milliseconds since the Unix epoch |
|---|---|
| SQLite | `CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)` |
| PostgreSQL | `(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT` |
| MySQL | `CAST(UNIX_TIMESTAMP(NOW(3)) * 1000 AS SIGNED)` |

It is inlined into the statement that guards the row, and `store_now_ms`
reads it on the executor carrying that statement rather than on a second
connection, so an expiry is written and compared on one clock and one
snapshot. The Redis scripts read `redis.call('TIME')` inside the script that
guards the key. A node whose clock runs fast can therefore neither extend a
lease, nor hide a live entry from its peers, nor declare a peer's record
elapsed. The `now_ms` arguments the `RebuildCoordinator` methods take are
node time and feed only the in-process coordinator.

Moving store time in a test needs a seam, because no test here sleeps.
`set_time_offset_for_test(offset_ms)` adds a fixed offset to the store clock
one adapter reads. It is per adapter instance rather than process-wide on
purpose, since several stores share one test binary and a process-global
offset would let one test's expiry move another's, and production always
passes zero. `RedisRenderStore` has no such seam and needs none: an entry's
lifetime there is Redis's own `PEXPIRE`, not a stored deadline this crate
compares against.

### Failure semantics

- A backend that is not there stops the boot, with one actionable sentence
  naming the migration or the variable to fix. Nothing is ever served against
  a missing table or an endpoint nothing answers.
- A backend lost at runtime is a store error. Every adapter maps a driver
  failure to `RenderCacheErrorKind::ProviderUnavailable` or
  `LedgerErrorKind::ProviderUnavailable`; the route's `FailurePolicy` then
  decides (open renders without caching, closed fails the request),
  coordinator errors take the existing provider-failure path, and ledger
  errors become Live provider failures.
- A lease is taken over once store time has passed its expiry. The former
  leader's `publish_token` answers `LeaseFenced` and its render is discarded.
- A record store `Conflict` that survives one retry is `Contention`.
- Eviction, expiry, or a restart of Redis makes entries miss and instances
  missing. That is fresh-render recovery, never reconstructed authority: the
  coherence check against the database generation ledger runs on every hit
  regardless of which L1 served the bytes.
- Bytes in a row or a hash are the signed codec frame, so a torn, truncated,
  or tampered value fails its integrity check and is a miss rather than a
  served page.

The two record stores differ in one contracted way. `SqlInstanceRecordStore`
joins the host's ambient transaction when one is open, so a claim taken
inside a request that rolls back leaves no row, which is the coupling a
database tier is chosen for. `RedisInstanceRecordStore` cannot join one and
does not pretend to: a Redis-backed ledger claims the successor first and the
host's effects follow. Specification 05 allows either coupling.
`SqlLeaseStore` joins nothing either, deliberately: a lease taken inside
someone else's transaction would become visible to other nodes only at that
transaction's commit, and a rollback would silently drop a lease a peer had
already observed as held.

### No cross-node waiting

A key another node holds is a `Bypass`: the request renders and publishes
nothing. Nothing in this build polls, sleeps, or parks waiting for a peer's
lease. The rebuild contract permits bounded duplicate computation across
nodes and forbids two accepted publications, and the store's fence is what
forbids the second one, so waiting would buy avoided work at the price of a
request's latency depending on a process it cannot see.

### Telemetry and test seams

Telemetry is unchanged by the tiers: the six counter names and the closed
attribute sets listed under Operations below are the same at every profile,
and no counter, attribute, or label names a tier, a provider, or a backend.

The seams the tier tests reach:

- `set_time_offset_for_test` on `SqlRenderStore`, `SqlLeaseStore`,
  `SqlInstanceRecordStore`, `RedisLeaseStore`, and
  `RedisInstanceRecordStore`, described above.
- `RenderCache::clear_l0_for_test`, which empties L0 and leaves L1, the
  authority epoch, and the coordinator exactly as they were. It is the only
  way a test can prove that a later request was served from L1 rather than
  from memory.
- `render_cache::sweep_l1(l1, now_ms, epoch)`, the body of
  `RenderCache::sweep` with the runtime lookup lifted out, so a test can
  drive a provider's own reclamation without binding the process singleton.
- `live::verify_ledger_driver_for_test(driver)`, the ledger probe over a
  driver a caller names rather than the bound runtime's, for the same reason.
- `ledger_conformance::run_all` and `run_two_node` in
  `suprnova-live-test-support`, the provider conformance suite written
  against `LiveInstanceLedger` alone, so the same scenarios run over the
  engine's kernels and over a framework adapter against a real backend.
- `RedisPrefixGuard` in
  `framework/tests/support/render_cache_tiers_support/`, which deletes
  everything under one test's key prefix on drop, so a failed assertion
  cannot leave keys behind; and in `framework/tests/render_cache/tiers.rs`,
  `OffsetRecordStore`, the small trait that lets one generic conformance body
  run over either record store, with `MirroredClockStore` keeping the suite's
  `ControlledClock` and the store's own clock in step.

Multi-node behaviour is proved at the provider layer, because the RenderCache
runtime is a process singleton: two adapter handles over one backend stand in
for two nodes. The middleware is proved end to end once per distributed
profile on top of that. The Database profile runs in the default suite on a
single-connection SQLite pool, which is what makes it prove something no
larger pool could: a coordinator or publish call made inside the render's own
read transaction would deadlock there rather than merely be slower. The Redis
profile runs in the `#[ignore]`d `live_redis_*` tests, which
`scripts/check-redis.sh` runs against a disposable `redis:7-alpine` container
on a Docker-assigned loopback port; `scripts/check-postgres.sh` and
`scripts/check-mysql.sh` each gained a tiers block that runs and asserts by
name on the `live_postgres_*` and `live_mysql_*` twins.

### Limitations

Each of these is ruled behaviour, not a defect.

- An instance record stays countable for 60 seconds after the instance
  lifetime the node clock measures runs out. The gap is deliberate: it is the
  same clock-skew allowance the rest of the crate gives, and it is what lets
  a request arriving just after an instance elapsed be told
  `RefreshReason::InstanceExpired` rather than `RefreshReason::Missing`. The
  cost is that an elapsed instance occupies configured capacity for that
  window. A promotion reservation gets no such window, because its retry
  identity has to be free the instant it elapses.
- Capacity is counted and then admitted, and across nodes those are not one
  atomic step, so N nodes creating instances at once can over-admit by at
  most N-1 against the configured `max_instances`. Each node's own count is
  exact: both stores count exactly the records whose store deadline has not
  passed, which is why an elapsed but still retained record is counted and a
  record past the retention window is not.
- Reclamation of records past their store deadline is bounded to 64 per
  creating operation, on SQL and on Redis alike, the same number the
  in-memory reference store uses. A burst of more than 64 due deadlines
  leaves the surplus rows or index members for the operations that follow;
  nothing reads or counts one waiting to be reclaimed. The database L1 has no
  automatic sweep of its own and is reclaimed only through
  `RenderCache::sweep`; the Redis L1 needs none, because every entry it
  stores carries a `PEXPIRE` and Redis reclaims the bytes itself.
- Precise duplicate-key classification on MySQL needs 8.0.19 or newer. Older
  MySQL and MariaDB report `for key 'PRIMARY'` without the table prefix
  8.0.19 added, and this build refuses to read a message it cannot attribute
  to the statement's own table, so a genuine collision there degrades to
  `ProviderUnavailable`. That is the safe direction, since a caller told the
  store failed retries or reports while a caller told a peer holds the key
  stops looking, and nothing is granted twice either way.
- The Redis adapters target a single Redis 7 or newer instance. The scripts
  touch keys they do not declare in `KEYS` (a reclamation pass deletes the
  index members its own range read found), which Redis Cluster refuses, and
  they read the store clock with `TIME` inside a script, which older servers
  refuse.
- `RedisRenderStore::inspect` is bounded rather than exhaustive: it reports
  what a capped `SCAN` found, stopping at 10,000 keys or 1,000 rounds, and
  says that it stopped. `SCAN` guarantees only that a key present for the
  whole scan is returned at least once, so the count is approximate in both
  directions rather than a floor.
- `max_bytes` on the database and Redis L1 bounds one entry, checked before
  any statement or command runs. It bounds neither the table nor the
  keyspace, and neither tier evicts to make room: the storage is shared by
  every node, so no single process holds an accurate picture of it, and
  growth is bounded by retention instead. Only the file tier bounds a whole
  directory, which it can because it owns that directory alone.
- A lease row and a lease key are never deleted, and neither is the
  publication token counter. Releasing sets the expiry to zero and leaves the
  row or hash where it is, because a restarted tenure counter or token
  counter would let a fenced-out leader's already-minted token outrank the
  publication that replaced it. The lease keyspace is therefore bounded by
  the number of distinct render keys ever rebuilt, not by the number
  currently held.
- A Redis-backed Live ledger never joins a host transaction and claims the
  successor before the host's effects. Specification 05 allows either
  coupling; a deployment that needs a claim to disappear with a rolled-back
  request chooses the database tier, whose record store joins the ambient
  transaction and therefore leaves no row when the host rolls back.
- A bypassing node renders without publishing. There is no cross-node waiting
  anywhere in this build, so a key another node is rebuilding costs this node
  one duplicate render. Bounded duplicate computation across nodes is
  accepted; two accepted publications are not.
- Losing Redis loses stored bytes and instance records; it never loses
  authority, and it can never make stale content current. Every hit is
  checked against the database generation ledger whatever served it.
- `SqlLeaseStore` always opens a short transaction of its own, and the
  middleware calls `admit`, `publish_token`, and `release` outside any host
  transaction. Both facts are load-bearing together, and both are proved by
  the Database-profile middleware test on its one-connection pool, where
  either being false would deadlock.
- `instance` and `idempotency` are `VARCHAR(64)`, sized for the 16- to
  32-byte identities this build issues carried as hex. A longer identity
  would need a migration.
- `LedgerError::new` is public. A host adapter has to be able to report a
  store failure or a capacity refusal without engine help, and the
  alternative, a second error type at the port, would have made every kernel
  translate.
- The Live ledger's fail-closed probe is a separate step,
  `live::verify_ledger_backend`, which `Server::run` calls before any request
  is served. It cannot live inside `LiveRuntime::bind`, which is synchronous
  and is reached from synchronous public constructors and from tooling, so a
  host that assembles a Live runtime without going through `Server::run` gets
  no probe.
- The store-time offset seam is per adapter instance and is never set in
  production. It exists because a test here does not sleep, and a
  process-global offset would let one test's expiry move another's.
- `cargo test --test render_cache` run plainly reports two known `privacy::*`
  failures. They predate this work and come from the shared-process runner
  rather than from the tests; `cargo nextest`, the runner the gate uses,
  gives each test its own process and is green.
- Deferred: credible generation hints over Redis pub/sub, captured in
  `iterations/next/redis-generation-hints.md`; Memcached; Redis Cluster;
  cross-node waiting; and separately cached nested segments, captured in
  `iterations/next/nested-cached-segments.md`.

## Operations

### File layout and the tally/disk invariant

L1 stores one file per key, flat under the configured directory:
`<key.to_base64url()>.snrc`. Each file holds one frame (the entry bytes
inside it carry a JSON header whose enum tags are `snake_case`, such as
`"class": "public_shared"` and `{"private": ..}`, matching every other JSON
name in the crate):

| Field | Bytes |
|---|---|
| magic `SNRF` | 4 |
| fence epoch | 8 |
| fence token | 8 |
| fence generation digest | 32 |
| published_at_ms | 8 |
| retention_ms | 8 |
| entry length | 4 |
| entry bytes | entry length |
| SHA-256 of everything before it | 32 |

Publication writes a temporary file, `fsync`s it, renames it over the
target, then `fsync`s the parent directory, so a reader only ever sees the
previous complete file or the new complete file, never a partial one,
across a crash or power loss. `FileRenderStore::open` scans the directory
once, rebuilding an in-memory byte tally that every later `publish`,
`evict`, and `sweep` call keeps in step with the directory, so none of them
ever needs to re-read the directory from disk; it also removes any leftover
`.tmp` file (a crash between creation and rename) and any `.snrc` file that
fails its frame check (wrong magic, a truncated or tampered body, a bad
digest), treating a torn write as self-healing rather than a permanently
poisoned entry. The one place this invariant runs in reverse is `sweep`'s
handling of a candidate whose file is already gone: that removal is not
counted (nothing on disk changed because of this call), but the tally entry
is still dropped, since disk is corrected into the tally rather than the
other way around for that one case.

`sweep(now_ms, epoch)` removes at most 64 entries per call (oldest
`published_at_ms` first), holding the tally lock across every removal. An
entry is dead when its age since publication reaches its stored
`retention_ms`, or its fence epoch is older than the current epoch;
`retention_ms` is set at publish time from the same class-aware Dead edge
`evaluate_freshness` uses (see Generations and coherence above), so a
private entry's file is retired earlier than a public entry's, and a sweep
can never disagree with a live freshness check about whether an entry is
truly dead. A removal that finds the file already gone is not counted, but
still corrects the in-memory tally to match the disk. `sweep` runs
automatically every 256th publication and returns a `SweepOutcome`
(`removed`, `more_remain`) so a backlog larger than the per-call limit
drains incrementally across later triggers or an operator's own explicit
call. `RenderCache::advance_epoch` clears L0 outright and immediately,
since it is in-process memory with no filesystem to reconcile against;
L1 is not touched by an epoch advance and keeps every pre-epoch file until
`sweep` reclaims it, bounded the same way.

### Environment variables

`RenderCacheConfig::from_env` reads:

| Variable | Default |
|---|---|
| `RENDER_CACHE_ENABLED` | `true` (anything but `false` or `0`) |
| `RENDER_CACHE_L0_ENTRIES` | 4,096 |
| `RENDER_CACHE_L0_BYTES` | 128 MiB |
| `RENDER_CACHE_L1_DIR` | unset (L1 disabled) |
| `RENDER_CACHE_L1_BYTES` | 1 GiB |
| `RENDER_CACHE_FAILURE` | `open` (`closed` is the only other accepted value) |
| `APP_BUILD_ID` | the application's own `CARGO_PKG_VERSION` |

It also reads the deployment-profile variables (`RENDER_CACHE_PROFILE`,
`RENDER_CACHE_L1`, `RENDER_CACHE_COORDINATOR`, `RENDER_CACHE_REDIS_URL`,
`RENDER_CACHE_REDIS_PREFIX`, `RENDER_CACHE_LEASE_MS`, and
`RENDER_CACHE_MAX_WAITERS`), which are tabled with their defaults under
Deployment tiers and providers above, beside the Live instance ledger's own
`LIVE_LEDGER_DRIVER`, `LIVE_REDIS_URL`, and `LIVE_REDIS_PREFIX`.

### Telemetry

Six closed counter names: `suprnova.render_cache.lookups`,
`suprnova.render_cache.hits`, `suprnova.render_cache.publications`,
`suprnova.render_cache.rebuilds`, `suprnova.render_cache.stitch.assemblies`,
and `suprnova.render_cache.stitch.slots`. `lookups` and `hits` carry the
`outcome` attribute with the eight `LookupOutcome` values listed under
"Framework middleware and policy" above (`l0`, `l1`, `conditional`,
`stale`, `miss`, `bypass`, `moved`, `declined`); `hits` increments only for
`l0`, `l1`, `conditional`, and `stale`. The two stitch counters carry their
own closed `outcome` sets: `assembled` and `fail_document` for assemblies,
`rendered`, `omitted`, `fallback`, and `failed` for slots. `publications`
and `rebuilds` are plain counts with no `outcome` attribute in this build.

### Console commands

Two hidden operator commands, registered the same way `crate::live::tooling`
registers its own: neither ever prints a stored body or a raw key.

- `render-cache:epoch-advance` calls `RenderCache::advance_epoch` and prints
  `epoch advanced to {epoch}`.
- `render-cache:inspect <key>` calls `RenderCache::inspect` with the given
  encoded key and prints the `EntryInspection` debug form plus the current
  epoch, or `no entry (current epoch: {epoch})` when the key names nothing
  stored. Both commands propagate a real failure (an unparseable key, or no
  runtime installed) as a command error rather than reporting success.

### What this build leaves out

- **Separately cached nested segments (fragment caching shared across
  documents).** A stitched entry's slots are re-rendered per request and
  never cached themselves, and a cached segment cannot contain another
  cached segment; captured in `iterations/next/nested-cached-segments.md`.
- **Memcached, Redis Cluster, and cross-node waiting.** The database and
  Redis tiers ship (see Deployment tiers and providers above), but the Redis
  adapters target a single Redis 7 or newer instance, no other network
  key/value backend has an adapter, and a key another node is rebuilding is
  a bypass rather than a wait.
- **Credible generation hints.** Nothing publishes or listens for a signal
  that a generation advanced, so a validation lease is shortened by nothing
  but its own policy; captured in
  `iterations/next/redis-generation-hints.md`.
- **The budget harness (plan D).** RenderCache has no benchmark harness of
  its own, unlike the checked-in snapshot, action, upload, and asynchronous
  budgets.
- **Session identity read versus session content read.** `Auth::id()`'s
  fallback to `session()` for an anonymous visitor records a session read,
  which always narrows to `Uncacheable`, even though the fallback only ever
  resolves identity; distinguishing that from a render that reads actual
  session content is parked as a next-iteration capture.
- **Feature-flag dependency generations.** Nothing advances a
  `DependencyIdentity::Feature` generation on a flag change or an
  evaluator reload, so a published entry that depended on a flag's answer
  does not get invalidated by that change; this gap, including that
  `DatabaseEvaluator::reload()` does not notify either, is parked as a
  next-iteration capture.
- **Authorization reads recording the identity consulted.** `Gate::allows`
  always maps to the `Principal` dimension regardless of what it actually
  checked, so a route whose gate is genuinely per-tenant cannot cache under
  `Tenant` alone; having `Gate` record what it consulted is parked as a
  next-iteration capture.
