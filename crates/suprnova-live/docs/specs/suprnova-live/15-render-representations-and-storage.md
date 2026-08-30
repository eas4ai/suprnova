# Suprnova Live -- 15 Render Representations and Storage

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the RenderCache entry model, route/group cache policy,
representation lookup, provider-backed L0/L1 storage, HTTP validators and cache
metadata, storage lifecycle, and the cache-hit response path. It depends on
canonical documents and feeds variance/composition and coherence. It is a
framework-managed response cache distinct from Suprnova's generic application
cache.

## Capabilities

### RenderCache policy and eligibility

Applications shall opt routes or route groups into an explicit RenderCache
policy. The framework shall determine whether a concrete response remains
eligible after observing method, status, headers, identity use, dependencies,
cookies, and other safety signals.

Acceptance criteria:
- GET and HEAD canonical representations are the default eligible methods;
  state-changing Live endpoints are ineligible.
- Policy declares representation class, freshness bounds, stale behavior,
  storage layers, coherence mode, and explicit bypasses.
- Statuses, redirects, errors, streaming bodies, cookies, and exceptional
  headers have documented eligibility rules.
- A handler can deliberately bypass or decline storage without poisoning an
  existing valid entry.
- Suprnova Live remains fully functional when RenderCache is disabled,
  unconfigured, bypassed, or unavailable; caching changes avoided work, not
  application capabilities.
- Framework safety may downgrade shared caching to private, stitched, or
  uncacheable behavior; unsafe developer configuration fails closed.
- Route/group inheritance and override precedence are deterministic.

UX flow:
1. Application developer opts a route or group into RenderCache -> checking
   exposes its effective policy.
2. A concrete response violates eligibility -> it is served normally but not
   stored under an unsafe representation class.

### Complete and Composite representation models

A RenderCache entry shall be explicitly typed as a Complete representation or a
Composite representation. A Complete representation stores finished immutable
HTTP body bytes and replayable metadata and may include public seed-backed Live
islands. A Composite representation stores an integrity-protected structural
segment graph with typed stitch slots and requires request-time assembly before
final response headers and validators exist. Neither type requires ORM
hydration, application handler execution, or JSON value deserialization merely
to read stored cache data.

Acceptance criteria:
- Both types include canonical key identity, timestamps/policy, variance,
  dependency observations, coherence metadata, format version, and a structural
  integrity proof appropriate to their form.
- A Complete entry includes status, final safe headers, content type/encoding,
  immutable body bytes, and the final represented-byte validator.
- A Complete entry containing public seeds records their earliest promotion
  deadline as a serving constraint independent of dependency coherence.
- A Composite entry includes segment/slot identities, assembly policy,
  provisional safe metadata, and a structural validator that is never replayed
  as the final assembled HTTP validator.
- Hop-by-hop, per-connection, transient tracing, and unsafe per-request headers
  are never replayed from storage.
- Complete body bytes correspond exactly to their final validator and metadata;
  Composite final bytes, `Content-Length`, CSP data, and validators are computed
  only after successful assembly.
- Entry decoding is bounded and versioned.
- Corrupt, incomplete, oversized, or unsupported entries are treated as misses
  and quarantined/evicted as appropriate.
- Cache metadata can be inspected without decoding or logging private body
  content.

UX flow:
1. Valid representation is stored -> future eligible requests directly reuse a
   Complete response or assemble a Composite without rerunning its public
   handler/template work.
2. Stored entry fails structural validation -> request follows miss/rebuild
   behavior and diagnostics identify the cache defect safely.

### Canonical lookup identity

RenderCache lookup shall begin from canonical route identity, normalized route
parameters, negotiated representation, application/view version, and the
variance contract owned by the privacy spec. Raw URLs, arbitrary cookies, and
unstable process-local identifiers shall not define keys accidentally.

Acceptance criteria:
- Route identity remains stable across equivalent URL spellings according to
  router canonicalization.
- Query parameters are normalized according to declared route semantics and do
  not collapse meaningfully different values.
- Host/scheme participation follows trusted deployment and routing policy.
- Key material is hashed or encoded with purpose separation and bounded length.
- Key format version changes cannot collide with prior entries.
- Developers can inspect safe decoded key dimensions without seeing sensitive
  values.

UX flow:
1. Eligible request reaches lookup -> framework derives one canonical base key
   plus safe variance dimensions.
2. Required dimension is ambiguous or unsafe -> lookup bypasses rather than
   risking a false hit.

### Provider-backed L0 and L1 storage

RenderCache shall use a binary/raw storage path suitable for immutable bytes and
typed Composite data. L0 shall provide in-process immutable storage. L1 shall be
selected through a RenderStore provider that may use the local filesystem, the
application database, Redis, Memcached, or another conforming networked
key/value cache without routing through the generic JSON-value cache contract or
requiring an external daemon for the embedded deployment tier.

Acceptance criteria:
- L0 can return shared immutable bytes without per-hit body serialization or
  avoidable copies.
- L1 stores and retrieves versioned Complete or Composite entries and metadata
  through capability-checked providers.
- Layer population, promotion, size limits, TTL/retention, and eviction are
  explicit.
- A slow or failed L1 does not corrupt L0 or the served response.
- Backend adapters preserve atomic entry publication and reject torn writes.
- Provider choice changes storage, coordination, and performance rather than
  Live or RenderCache correctness semantics.
- Generic `suprnova::Cache` semantics remain unchanged for application values.

UX flow:
1. Request hits L0 -> Suprnova validates coherence and either sends Complete
   bytes or assembles the stored Composite without invoking public route work.
2. L0 misses and L1 hits -> the valid typed entry promotes under policy and
   serves/assembles; total miss enters the rebuild contract.

### HTTP caching and conditional requests

RenderCache shall generate and replay correct HTTP cache metadata so browsers,
reverse proxies, and CDNs can participate without becoming the authority for
server coherence.

Acceptance criteria:
- Strong or weak ETag policy is explicit and computed from the represented
  variant.
- `If-None-Match` and applicable conditional headers can return 304 with correct
  metadata and no body.
- `Cache-Control`, `Vary`, surrogate directives, age, and private/public markers
  agree with variance and coherence policy.
- Responses default to `Cache-Control: private` with respect to shared external
  caches unless the route explicitly selects bounded `s-maxage` semantics or a
  supported purge/invalidation contract.
- External freshness/stale directives for seed-bearing Complete entries fit
  within the remaining seed acceptance window at issuance.
- Private or stitched output is never labeled publicly reusable.
- Content encoding variants have distinct validators where their bytes differ.
- Conditional handling for Complete entries can succeed without assembly;
  Composite handling uses only a validator computed for the final assembled
  recipient-specific bytes.

UX flow:
1. Browser presents a matching validator -> Suprnova returns 304 when the stored
   representation remains coherent.
2. Validator is stale or belongs to another variant -> normal lookup/rebuild
   returns the correct current representation.

### Cache hit, miss, and bypass execution

The request pipeline shall make hit, miss, stale, and bypass behavior explicit
and observable. A proven Complete hit shall bypass handler, database, ORM
hydration, serialization, and template work. A proven Composite hit shall
bypass the stored public handler/render work while executing only the declared
request-specific slot pipeline.

Acceptance criteria:
- Lookup occurs after middleware required for safe routing/identity/variance and
  before application work that the hit can avoid.
- A Complete hit reconstructs only approved final metadata and immutable body;
  a Composite hit loads validated graph metadata before bounded assembly.
- A Complete hit follows the direct immutable-byte path; a Composite hit invokes
  only bounded slot rendering and assembly required by its stored graph.
- Miss and bypass continue through the ordinary route pipeline exactly once.
- Successful eligible rendering publishes atomically after all metadata and
  coherence observations are complete.
- Cache lookup or storage failure follows declared fail-open/fail-closed policy
  without serving an unproved representation.
- Tracing distinguishes L0, L1, conditional, stale, miss, bypass, and rebuild.

UX flow:
1. Valid entry exists -> application user receives the same canonical document
   with minimal server work.
2. No valid entry exists -> route renders normally and may populate RenderCache;
   interaction semantics do not change.

## Acceptance criteria

- RenderCache is distinct from generic application caching and stores typed
  versioned Complete or Composite representations.
- Route eligibility and concrete-response safety both govern storage.
- Keys are canonical, bounded, inspectable, and variance-aware.
- Complete hits avoid route, ORM, and template execution; Composite hits avoid
  cached public work while executing only required private/request-specific
  slots.
- Browser/CDN validators never weaken server-side coherence or privacy.

## Decisions and revisions

- 2026-08-21 -- RenderCache is a framework layer above `suprnova::Cache`, with a
  binary/raw storage path rather than JSON string serialization.
- 2026-08-21 -- A proven hot hit should approach immutable static-response
  economics by avoiding application work.
- 2026-08-21 -- Split entries into directly sendable Complete representations
  and assembly-required Composite representations. Public seed-backed islands
  may remain in Complete bytes.
- 2026-08-21 -- L1 is provider-backed across file, database, and conforming
  network key/value stores. Live and RenderCache require no external daemon at
  the embedded tier.
