# Suprnova Live -- 18 Cache Coherence and Rebuilding

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns proof that a stored representation remains valid, the
authoritative dependency-generation ledger, local validation leases,
deployment tiers and provider selection, multi-node propagation, fresh/stale
policy, singleflight rebuilding, fencing, failure handling, deployment
evolution, and coherence observability. It depends on representation storage
and dependency tracking. It does not own cache privacy classification or
application domain transactions.

## Capabilities

### Authoritative coherence validation

Before reuse, RenderCache shall prove that the entry's format, representation
identity, variance, application/view version, and recorded dependency
generations remain compatible with current authority. Expiration alone shall
not be treated as proof of data correctness.

Acceptance criteria:
- Validation algorithm and authority sources are explicit and deterministic.
- Any required generation mismatch makes the entry stale/invalid according to
  policy.
- Missing, unreadable, reset, or incompatible generation authority fails closed
  or serves stale only under an explicit bounded stale-on-error policy.
- Validation can reject an entry before body retrieval where metadata permits.
- A valid TTL cannot override a known generation mismatch.
- Uncacheable/private classification changes invalidate incompatible reuse.

UX flow:
1. Request finds a stored entry -> coherence compares its observations to
   current authority.
2. Proof succeeds or fails -> entry serves immediately or enters stale/rebuild
   behavior without guessing.

### Complete deployment tiers and provider contracts

Live and RenderCache shall expose the same application-facing behavior and
correctness guarantees without requiring an external cache daemon. Deployment
tiers select implementations of four contracts: RenderStore,
LiveInstanceLedger, RebuildCoordinator, and GenerationLedger. Generation hints
are an optional capability and may be a no-op. Application component, route,
and cache-policy code shall not change between tiers.

Acceptance criteria:
- Tier 0, Embedded, supports one process with memory L0, memory instance ledger
  and local rebuild coordination, file-backed L1, and generation truth in the
  application's existing database or bundled embedded SQLite where required.
- Tier 1, Database-coordinated, supports a small multi-node deployment using the
  shared application database for the instance ledger CAS, fenced lease rows,
  generation truth, and optional blob-backed L1, with no cache daemon or pub/sub
  requirement.
- Tier 2, Externally accelerated, may use Redis, Memcached, or another conforming
  networked key/value provider for L1 bytes, instance records, and rebuild
  coordination, with optional Redis-style hints; generation truth remains in
  the database.
- Tier 0 is complete, not degraded: loss of distribution changes topology and
  performance, never Live features, privacy, authority, accepted-outcome, or
  cache-correctness semantics.
- Tier 1 documents that an ordinary Live action performs a database instance
  CAS/write; applications move coordination to Tier 2 only when that traffic or
  cache-byte workload becomes material.
- RenderCache may be disabled entirely and Live continues through ordinary
  rendering with no cache provider or generation-ledger dependency.
- Missing or evicted volatile provider state fails as miss, refresh-required, or
  rebuild; it never recreates revision/generation authority from old client or
  cache data.
- Tier 0 is the behavioral reference implementation for provider conformance;
  Tier 1 and Tier 2 additionally pass multi-node, eviction, partition, lease,
  fencing, and failure tests applicable to their advertised capabilities.

UX flow:
1. Application developer selects a deployment tier -> configuration wires the
   conforming providers without changing application code.
2. An optional provider is absent or fails -> Live remains usable and cache work
   bypasses, misses, refreshes, or rebuilds according to the same safe contract.

### Durable generation ledger

The owning application database or databases, including SQLite, shall hold
durable logically append-only generation truth at every cache deployment tier.
A database-free application may run Live with RenderCache disabled; an embedded
cached deployment may use bundled SQLite for cache metadata. Redis, Memcached,
pub/sub, and process memory may accelerate observation but shall not become
generation correctness authorities merely because they are fast.

Acceptance criteria:
- Ledger schema/namespace, logically append-only sequence semantics, atomic
  transaction coupling, backup/restore, compaction, migration, and authority
  epoch are defined for each supported database.
- Multi-node readers observe committed values under the selected consistency
  contract.
- Network key/value-provider loss, eviction, restart, or hint loss cannot make
  stale data appear proven current.
- Authority unavailability follows explicit stale-on-error or fail policy.
- Reconciliation detects drift between write sources and ledger state.
- Operational tooling can inspect safe generations and epochs.

UX flow:
1. Application write commits -> durable authority records advancement and may
   publish acceleration hints.
2. An accelerator misses the hint -> later authoritative validation still
   discovers the change.

### Local validation leases and invalidation hints

L0 caches may use short bounded validation leases to avoid querying authority on
every hot hit. Tier 0 and Tier 1 require no pub/sub; they reread database
generations when leases expire. Tier 2 may invalidate leases early through
credible generation hints. Leases shall define maximum staleness independently
of hint availability.

Acceptance criteria:
- Lease duration is policy-controlled per coherence class and included in
  measurable staleness bounds.
- A hint can invalidate local generation/entry state but cannot prove validity
  beyond authority.
- Node restart begins without assuming prior lease validity.
- Clock behavior, monotonic time, and skew assumptions are documented.
- Security-sensitive/private representations can require stricter validation.
- Lease/hint failure metrics expose effective validation age.

UX flow:
1. Hot node reuses a still-valid local lease -> it serves entries within the
   policy's explicit validation-age/staleness bound without repeated authority
   reads.
2. Write hint or lease expiry occurs -> next request revalidates before claiming
   freshness.

### Singleflight and fenced publication

One fenced winner per cache key and coherence epoch shall be permitted to
publish a rebuilt representation. Local failure, lease expiry, or distributed
partition may cause bounded duplicate computation, but never two accepted
publications for the same fence. Other requests shall wait, receive allowed
stale content, or bypass according to explicit limits.

Acceptance criteria:
- In-process and distributed singleflight coordinate without deadlock or
  unbounded waiter growth.
- Tier 0 uses local coordination, Tier 1 uses database lease rows with
  database-time expiry and monotonic fencing, and Tier 2 may use a conforming
  key/value coordinator.
- Distributed ownership uses expiry and fencing so an old slow rebuilder cannot
  overwrite a newer result.
- Rebuild captures a fresh coherent dependency set and publishes atomically.
- Wait timeout, cancellation, leader failure, and node shutdown have defined
  takeover behavior.
- A rebuild never holds database locks or request resources longer than policy
  permits accidentally.
- Load tests prove invalidation storms do not create equivalent render/DB storms.

UX flow:
1. Many requests observe one invalid entry -> one fence owns publication while
   duplicate computation, if failure forces it, remains bounded.
2. Other requests -> receive permitted stale content, await bounded completion,
   or follow bypass policy without duplicating the herd.

### Freshness and stale-while-revalidate

Each coherence policy shall define fresh, stale-servable, stale-on-error, and
dead intervals independently from dependency validity. Known dependency change
may move an entry to stale immediately; serving it remains an explicit business
decision, not a claim of freshness.

Acceptance criteria:
- Fresh and stale windows, allowed status/classes, background rebuild, and
  maximum staleness are explicit.
- Stale responses carry accurate HTTP warning/age/cache metadata where required.
- Private, authorization-sensitive, destructive, or legally sensitive content
  can prohibit stale serving.
- One background publication fence exists per key/epoch; failure may cause only
  bounded duplicate computation.
- A dependency mismatch is observable even when stale serving is permitted.
- Dead entries are never served and are eligible for eviction.

UX flow:
1. Entry becomes stale under an allowed policy -> application user receives a
   bounded old representation immediately while one rebuild runs.
2. Rebuild completes -> subsequent requests receive the new coherent content.

### External shared-cache boundary

Browser, reverse-proxy, and CDN caches cannot consult Suprnova's generation
ledger while serving a stored response. Suprnova shall therefore expose shared
external caching only through an explicit bounded-staleness or supported purge
contract rather than describing external reuse as generation-proven current.

Acceptance criteria:
- Responses default to private with respect to shared external caches unless a
  route explicitly opts into bounded `s-maxage` or a supported surrogate purge
  policy.
- Browser/private cache freshness uses `no-cache`/revalidation or an explicit
  bounded `max-age`; `private` prevents shared reuse but does not itself prove
  current origin generations.
- External freshness and stale bounds do not exceed the route's declared
  business/privacy tolerance and remain honest when origin generations move.
- A purge hook is an acceleration/reduction mechanism unless its delivery and
  acknowledgement contract is strong enough to be named as authority.
- Origin-side generation validation resumes whenever the request reaches
  Suprnova; external age never proves origin coherence.
- Private, identity-bound, stitched, legally sensitive, or
  authorization-sensitive responses fail closed against shared storage.
- Tests cover data changes while a response remains at an external cache and
  assert the configured staleness or purge behavior.

UX flow:
1. Route opts into external sharing -> clients may receive content within its
   explicit bounded stale window without a claim of generation proof.
2. The route requires strict current authority -> shared external caching stays
   disabled and requests revalidate through Suprnova.

### Rebuild and authority failure

Cache, ledger, route, database, renderer, and publication failures shall have
separate policies. A failure shall never publish incomplete output, extend
staleness without bounds, or make an unverified entry appear fresh.

Acceptance criteria:
- Last-known-good content may serve only within its explicit stale-on-error
  bound and privacy class.
- Failed rebuild preserves the prior atomic entry and releases/times out its
  singleflight ownership safely.
- Backoff and circuit behavior prevent continuous failing rebuild storms.
- Partial new metadata/body is discarded.
- Error responses are cached only under a separate explicit negative-cache
  policy.
- Application and operator diagnostics distinguish authority, storage,
  dependency, render, and publication failures.

UX flow:
1. Rebuild fails while safe stale content exists -> policy serves it with honest
   metadata and schedules bounded retry.
2. No safe representation exists -> ordinary route error behavior appears
   rather than fabricated cached success.

### Deployment and multi-node evolution

Rolling deployments, schema/view changes, key-format changes, and node restarts
shall invalidate or namespace incompatible representations without global
manual purges as the only safety mechanism.

Acceptance criteria:
- Application/view build identity and entry format version participate in
  validation/keying.
- Compatible rolling versions can share entries only under an explicit contract.
- Incompatible nodes cannot overwrite each other's formats under the same key.
- Global epoch bump provides a bounded emergency invalidation path.
- Multi-node tests cover propagation loss, partitions, clock skew, configured
  network key/value provider loss, leader death, and overlapping deployments.
- Old unreachable entries expire/evict without correctness-dependent scanning.

UX flow:
1. New deployment changes rendering contract -> new requests ignore or rebuild
   incompatible entries automatically.
2. Nodes overlap during rollout -> each serves only representations it can prove
   compatible and coherent.

## Acceptance criteria

- Entry reuse is proven from current authority, not TTL or best-effort deletion.
- Durable generation truth survives loss of accelerators and process state.
- Leases and hints reduce validation cost within explicit staleness bounds.
- Singleflight and fencing guarantee one accepted publication while bounding
  duplicate computation and preventing stale overwrites.
- Embedded, database-coordinated, and externally accelerated tiers preserve the
  same application-facing semantics without mandatory daemons.
- Fresh/stale/error/deployment behavior remains bounded, private-safe, and
  observable across nodes.

## Decisions and revisions

- 2026-08-21 -- The durable application database is generation authority at
  every tier; local memory and networked key/value providers are accelerators,
  not alternate generation truth.
- 2026-08-21 -- Stale-while-revalidate is policy-controlled and never erases a
  known dependency mismatch.
- 2026-08-21 -- Defined complete Embedded, Database-coordinated, and Externally
  accelerated tiers behind four provider contracts. Tier 0 is the behavioral
  reference and requires no external daemon.
- 2026-08-21 -- Singleflight promises one fenced publication, never universally
  one computation under distributed failure.
- 2026-08-21 -- External shared caches receive explicit bounded-staleness or
  purge semantics; their age cannot prove current origin generations.
