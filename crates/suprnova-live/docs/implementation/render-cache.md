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

The framework adapts those contracts to Suprnova, under
`framework/src/render_cache/`:

- `mod.rs`: the `RenderCache` facade (`install`, `bump_permission_version`,
  `advance_epoch`, `inspect`, `store_inspection`, `sweep`) and the
  process-installed flag that gates every write-side probe.
- `config.rs`: `RenderCacheConfig::from_env` and its environment variables.
- `collector.rs`: the request-scoped dependency collector, a Tokio task-local,
  and the process-wide permission-version counter.
- `middleware.rs`: `RenderCacheMiddleware`, the whole request flow.
- `file_store.rs`: `FileRenderStore`, the file-backed L1 provider.
- `ledger.rs`: `SqlGenerationLedger`, the database-authoritative
  `GenerationLedger` implementation, and its migration presence check.
- `orm.rs`: the write-side hooks that advance generations for supported ORM
  and query-builder writes.
- `migration.rs`: the RenderCache schema migration.
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
migration's tables are present, builds the Live key ring, assembles L0
(`MemoryRenderStore`, bounded by `config.l0`), L1 (a `FileRenderStore` when
`config.l1` names a directory, otherwise none), the clock, the rebuild
coordinator (a `LocalRebuildCoordinator` with a 30 second lease and 128
waiters unless overridden), and the SQL generation ledger; it then appends
`RenderCacheMiddleware` to the process-wide global middleware chain and
marks the process installed. Installation never clears the middleware
registry, so an application's own logging, session, CSRF, and auth
middleware are unaffected; it must run **after** every `global_middleware!`
registration that establishes request-scoped locale or identity, since the
middleware reads `Lang::locale()` and `Auth::id()` before the handler runs.
The process-installed flag also gates the write side: `orm::advance` and
`ledger::advance_in_current_transaction` consult it before issuing any SQL,
so an application that never installs RenderCache pays zero RenderCache SQL
on any write.

`RenderCache::bump_permission_version()` advances the process-wide counter
fed into `Principal` variance material. `RenderCache::advance_epoch()`
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
   database read transaction when a database is configured, so the
   generations it reads at window-close share one snapshot with the data the
   render itself read.
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
compare against:

- **Headers**, read through `Request::header` and friends, are deliberately
  not instrumented: every request reads some header for some purpose, so
  recording every read would decline every response.
- **Configuration**, read through `Config::get::<T>()`, has no producer: the
  call returns whole typed structs, so a read that touches secret
  configuration is indistinguishable at that seam from one that does not.
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

- **The consistent read view.** The leader's render runs inside
  `DB::transaction` when a database is configured, and the observation
  window closes (reading the ledger) while that transaction is still open,
  so the generations it reads share one snapshot with whatever data the
  render itself read. A write that lands after the transaction commits is
  therefore never visible to this reread as if it had already happened
  before the render started.
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
every mounted public-seed island, and a sticky `no_store` flag. Mount facts
are recorded from `LiveDocument::mount` itself, immediately after a mount
succeeds, rather than from `render` - a handler can mount an island and
hand-build its own response from `MountedIsland::html()` without ever
calling `render`, so recording at mount means the fact exists regardless of
whether `render` is reached. A rendered document's cache intent is recorded
separately, and only when it is `NoStore`: `Private` and `Public` intents
neither narrow nor widen this server-side cache's class, since
`DocumentResponseIntent::html()` defaults to `Private` and mapping that
default to `RepresentationClass::PrivateCached` would demote every Live
document with no `ClassificationReason` behind the demotion for the value
guard to check the key against. The route's own `RenderCachePolicy`, not
the document's intent, governs this server-side cache; the intent governs
only the downstream `Cache-Control` a browser or CDN sees.

`document_declines` can only decline a render, never narrow or widen what
`classify` already decided: any identity-bound island mounted in the
request, a document that declared `NoStore`, or a public-seed island whose
promotion deadline could not be resolved, all decline storage outright. A
seed's remaining time is also checked once more immediately before
publication (`seed_remaining_ms`); if the deadline is reached between the
render starting and this point, the candidate is declined rather than
stored already dead.

## Operations

### File layout and the tally/disk invariant

L1 stores one file per key, flat under the configured directory:
`<key.to_base64url()>.snrc`. Each file holds one frame:

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

### Telemetry

Four closed counter names: `suprnova.render_cache.lookups`,
`suprnova.render_cache.hits`, `suprnova.render_cache.publications`,
`suprnova.render_cache.rebuilds`. Only `lookups` and `hits` carry the
`outcome` attribute, with the eight `LookupOutcome` values listed under
"Framework middleware and policy" above (`l0`, `l1`, `conditional`,
`stale`, `miss`, `bypass`, `moved`, `declined`); `hits` increments only for
`l0`, `l1`, `conditional`, and `stale`. `publications` and `rebuilds` are
plain counts with no `outcome` attribute in this build.

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

- **Composite stitching (plan B).** `RepresentationClass::PublicShellStitched`
  and `EntryKind::Composite` exist as types, but no segment graph assembly,
  and no provider that writes or reads one, is implemented in this build.
- **Database and Redis tiers (plan C).** The only storage providers are the
  in-process `MemoryRenderStore` (L0) and the file-backed `FileRenderStore`
  (L1); there is no shared, cross-process, or cross-node tier.
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
