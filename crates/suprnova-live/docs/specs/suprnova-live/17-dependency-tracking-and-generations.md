# Suprnova Live -- 17 Dependency Tracking and Generations

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns cacheable-pipeline dependency collection, dependency
identities, monotonic generation values, ORM/query/model/config integration,
transactional logically append-only generation advancement, consistent render
observation, final publication rereads, granularity, and custom dependency APIs.
It depends on render contexts and cache variance and feeds cache coherence and
testing. Rebuild concurrency and deployment-tier validation belong to the next
spec.

## Capabilities

### Request-local dependency collection

After middleware needed to resolve safe routing, identity, and variance, the
entire opted-in cacheable handler and rendering pipeline shall execute inside a
collector that records authoritative inputs used to produce the representation.
Collection shall be automatic for framework-controlled reads and explicit for
application sources the framework cannot observe.

Acceptance criteria:
- The collector is request/task scoped, async-safe, nestable, and removed after
  render completion.
- Collection begins before cacheable handler/application data work and remains
  active through template rendering, metadata completion, and dependency-set
  sealing.
- Framework integrations cover ORM/query reads, model reads, route parameters,
  locale, feature/config values, view/application version, auth/session use, and
  declared services where applicable.
- Duplicate observations coalesce without losing the most conservative
  dependency.
- Reads outside an active opted-in collector retain ordinary behavior.
- Background tasks do not accidentally inherit and mutate a completed request's
  collector.
- Failed renders do not publish incomplete dependency sets.

UX flow:
1. Cacheable route handler or renderer reads data -> integrations register
   dependencies without manual tag lists.
2. Application uses an unknown external source -> explicit dependency API or
   safe cache bypass is required.

### Dependency identities and generations

Each dependency shall have a stable typed identity and monotonically advancing
generation controlled by its authoritative subsystem. Database-backed
generations shall have logically append-only committed change-event semantics:
the current generation is the latest committed sequence for the dependency,
even when a provider uses an equivalent compact physical representation.
Generations prove change since observation; they are not timestamps,
query-result hashes, or deletion lists.

Acceptance criteria:
- Dependency kinds and canonical identities are namespaced and versioned.
- Generation values never move backward within an authority epoch.
- Sequence allocation, index layout, compaction/pruning, and record-version
  optimizations preserve logical append-only ordering and are benchmarked per
  supported database rather than assumed contention-free.
- Missing, reset, overflow, migration, and authority-epoch changes have explicit
  invalidation behavior.
- Generation identity can represent table/model, record, relation/query class,
  configuration, feature, locale resource, or application/view version.
- Stored entries record exactly the generations observed at successful render.
- Generation values contain no private record content.

UX flow:
1. Render observes a dependency -> entry records its current typed generation.
2. Authoritative change advances that generation -> later coherence validation
   detects mismatch and rebuilds rather than relying on TTL guessing.

### ORM query and model dependencies

Suprnova ORM integration shall derive safe dependencies from actual reads.
Specific primary-key reads may use record granularity; complex or uncertain
queries shall conservatively depend on broader model/table generations.

Acceptance criteria:
- Query fingerprint includes normalized operation shape and bound-value meaning
  where used, but is never the sole proof that result rows are unchanged.
- Primary-key and known-relation reads can register record/relation identities.
- Filters, joins, aggregates, raw SQL, database functions, and unsupported
  queries fall back to documented conservative dependencies.
- Eager-loaded and lazy-loaded related data registers its own dependencies.
- Empty query results still depend on changes that could make them non-empty.
- Multi-database or tenant data sources remain namespaced.

UX flow:
1. Route reads a known record -> collector records the narrow safe dependency.
2. Route executes an opaque query -> collector records a broader generation so
   correctness wins over hit rate.

### Transaction-aware write advancement

ORM and model writes shall record relevant generation advances inside the
owning data transaction so the data change and generation change become visible
atomically at commit. Rollback shall retain neither, and multi-write
transactions shall publish a coherent advancement set.

Acceptance criteria:
- Create, update, delete, relationship changes, bulk operations, raw writes, and
  soft-delete/restore integrations declare affected dependency kinds.
- Generation events are written as part of the successful data transaction and
  become observable only after commit; they never survive rollback.
- Record updates may use an atomically changed row version where its semantics
  are equivalent; deletes, membership changes, empty-result queries, and broad
  query classes advance an authority that survives the affected row.
- A transaction can coalesce repeated changes to the same dependency.
- Advancement failure has an explicit transaction/outbox/reconciliation policy
  and cannot be silently ignored.
- Cross-process consumers observe committed advancement in causal order required
  by coherence policy.
- Manual database changes require documented capture, trigger, CDC, or global
  invalidation strategy.

UX flow:
1. Application commits a domain write -> relevant dependency generations advance
   atomically with the committed data under the coherence contract.
2. Transaction rolls back -> generations remain unchanged and valid cache
   entries are not discarded unnecessarily.

### Consistent render observation and publication reread

A rebuild shall associate rendered data with generations from a consistent
authority view and shall perform a fresh authoritative reread after that render
view closes. It shall publish only when every observed generation still matches.
A reread inside the same repeatable-read snapshot is not a valid final check.

Acceptance criteria:
- Database reads and their generation observations execute inside one declared
  consistent read snapshot or an equivalently proven authority mechanism.
- All dependency-producing lazy/template reads complete before that snapshot is
  closed and the dependency set is sealed.
- After rendering and closure of the consistent snapshot, a fresh autocommit or
  otherwise current authority read compares every sealed observation before
  publication.
- Any mismatch abandons the candidate and enters bounded rebuild/retry policy;
  incomplete bytes and metadata are never published.
- Multiple databases and external authorities provide equivalent version
  observation/final comparison or conservatively bypass caching.
- Tests force writes before, during, and after data reads, rendering, final
  reread, and publication to prove old data cannot be stored under a new
  generation.

UX flow:
1. Rebuild reads a coherent data/generation view -> rendering seals the observed
   dependency set.
2. A dependency moves before publication -> the fresh reread rejects the
   candidate and rebuild policy retries or bypasses without storing false proof.

### Granularity and conservative fallback

Dependency granularity shall be selected by proven observability, with
correctness before cache-hit optimization. Application developers may choose a
broader explicit dependency but shall not declare a narrower dependency than
the framework can verify.

Acceptance criteria:
- Table/model, record, relation, query-class, and custom granularity have stated
  safety preconditions.
- Unknown reads trigger broader dependency or bypass instead of false precision.
- High-cardinality dependency sets have limits and collapse to a safe broader
  generation.
- Granularity changes invalidate incompatible stored metadata.
- Diagnostics show why a broad fallback occurred and how an integration can
  improve it safely.

UX flow:
1. Collector can prove narrow identity -> entry gains precise invalidation.
2. Dependency count or uncertainty exceeds policy -> it collapses/bypasses
   conservatively and reports the optimization opportunity.

### Custom and external dependencies

Applications and integrations shall register typed dependencies and advance
their generations for external APIs, files, CMS content, secrets-derived public
versions, or other sources not owned by the ORM.

Acceptance criteria:
- Custom dependency namespace ownership prevents collisions.
- Registration and advancement APIs require explicit authority and versioning.
- External values use versions/epochs, not sensitive content, as dependency
  identity.
- Unavailable authority can force bypass or conservative invalidation.
- Tests can set and advance custom generations deterministically.
- Documentation warns that registering a dependency without advancing it is a
  correctness defect.

UX flow:
1. Render reads an external source -> integration records its current safe
   version.
2. Source changes -> owning integration advances the generation and future
   requests rebuild affected output.

## Acceptance criteria

- Opted-in renders automatically collect framework-controlled inputs.
- Typed generations, not SQL hashes or TTLs, prove whether dependencies changed.
- ORM reads and committed writes participate with conservative safe granularity.
- Rebuilds pair data with a consistent generation view and fresh post-render
  authority reread before publication.
- Rollbacks never advance generations and unobserved changes have an explicit
  strategy.
- Custom sources can join the same authority model without exposing content.

## Decisions and revisions

- 2026-08-21 -- Query fingerprints assist dependency identity but are not cache
  validity proof; data generations are authoritative.
- 2026-08-21 -- Automatic collection is the default inside opted-in rendering;
  explicit APIs cover sources Suprnova cannot observe.
- 2026-08-21 -- The collector wraps the cacheable handler and renderer, not only
  template execution.
- 2026-08-21 -- Database generation truth is logically append-only and commits
  atomically with owned writes. Physical layouts are provider-specific and
  benchmarked rather than presumed lock-free.
- 2026-08-21 -- Publication requires a fresh authority reread after the
  consistent render snapshot closes; rejected rereading inside the same
  repeatable-read view.
