# Suprnova Live -- 16 Cache Variance, Privacy, and Stitching

Status: Normative design specification
Last revised: 2026-09-09

## Scope

This domain owns the dimensions that distinguish rendered representations,
classification as public, private, stitched, or uncacheable, automatic privacy
downgrades, private-key design, server stitching of shared and request-specific
segments, Complete-versus-Composite classification, and leak prevention. It
depends on render contexts, security, and the RenderCache entry model and feeds
dependency/coherence validation.

## Capabilities

### Explicit variance model

Every cached representation shall declare the complete set of request and
application dimensions allowed to change its bytes or safe response metadata.
Variance shall use stable purpose-specific values rather than raw sessions,
arbitrary cookies, or implicit process state.

Acceptance criteria:
- Supported dimensions include canonical route/query parameters, host where
  meaningful, locale, negotiated media/encoding, tenant, principal, role or
  permission version, feature/config version, and explicit application values.
- Each dimension defines normalization, sensitivity, cardinality, keying, and
  invalidation behavior.
- `Vary` headers and server-side key dimensions remain consistent.
- Unknown cookies, headers, or context reads cannot silently affect shared
  output.
- High-cardinality and attacker-controlled dimensions have limits and
  diagnostics.
- Variance descriptors are stored and validated with the representation.

UX flow:
1. Rendering reads a declared variance source -> its normalized dimension joins
   the representation identity.
2. Undeclared request data affects output -> policy downgrades/bypasses and
   diagnostics identify the unsafe dependency.

#### Global-scope evaluation observes the tenant it reads

Global-scope evaluation that reads a tenant SHALL record a tenant observation
into the active collector, with the tenant as comparable material rather than a
bare reason. A route whose model carries a tenant-scoped global scope and which
declares no `Tenant` variance SHALL be declined rather than published, and the
privacy leak suite SHALL prove it with a positive control showing that the same
route declaring `Tenant` caches and partitions. A scope that reads no
per-request state SHALL record nothing and SHALL keep caching exactly as it
does now; a scope that reads per-request state the recording cannot resolve to
a known variance dimension SHALL narrow to `Uncacheable` rather than be assumed
harmless.

#### Media and Encoding negotiation

A route declaring `Media` or `Encoding` variance SHALL partition its stored
representations by the negotiated media type or content coding, drawn from a
closed set the route declares. The key, the `Vary` header, and the stored
representation SHALL agree on that value, and a negotiation result outside the
declared set SHALL fall back to the declared default rather than create a
variant. Tests SHALL prove that two negotiations yield two representations and
that a variant is never served to a request that did not negotiate it. Until
iteration 006 delivers this, the middleware resolves both dimensions to
constants before keying, so declaring either partitions nothing; that statement
of present behavior stands as the behavior iteration 006 replaces.

### Automatic privacy classification

RenderCache shall begin from the route's permitted class and automatically
downgrade when rendering observes identity, session, authorization, tenant, or
other private state. Automatic detection supplements explicit policy; it does
not justify promoting private output to public.

Acceptance criteria:
- Classes include public shared, public shell with private stitched segments,
  private cached, and uncacheable.
- Reading current principal, session values, private authorization, or secret
  feature context triggers the appropriate downgrade or explicit safe variance.
- A route cannot override an observed private dependency into public storage
  without a proven sanitizer/segment boundary.
- Anonymous and authenticated variants cannot collide.
- Logout, principal change, and permission version change prevent private reuse.
- Classification reasoning is inspectable in development and tests.

UX flow:
1. Application developer renders personalized content under a shared policy ->
   framework selects safe private/stitch/bypass behavior automatically.
2. Safety cannot be established -> response is served uncached rather than
   exposing one application user's content to another.

#### Authorization decisions record the identity they consulted

An authorization decision SHALL record the identity axis, and the concrete
identity, that the decision actually consulted, rather than only that a
decision ran. A decision that consults only a tenant-scoped fact SHALL classify
and key-compare as `Tenant` material; a decision that consults a per-user fact
SHALL keep requiring `Principal`; a decision that consults both axes, or one
the recording cannot resolve cleanly, SHALL keep the conservative behavior of
requiring `Principal` rather than guess. Role and permission reads that reach
the database SHALL observe their known tables precisely before a route that
evaluates one is stored, and the privacy leak suite SHALL gain a case proving
that a per-tenant-only gate does not leak one tenant's authorized page to a
different tenant sharing the same key.

### Private representation keys

Private caching shall use stable revocable purpose identifiers with bounded
cardinality and lifetime. Raw session IDs, bearer tokens, cookies, email
addresses, and other reusable secrets shall never become cache keys or metadata.

Acceptance criteria:
- Principal and tenant key material uses internal opaque identifiers or keyed
  derivation appropriate to threat policy.
- Permission or membership changes can invalidate previously permitted output
  without rotating every browser session.
- Private entries have bounded retention and eviction independent of public
  popularity.
- Key and diagnostic output cannot reveal sensitive identity data.
- Shared backends enforce namespace and environment isolation.
- Anonymous-private state such as a safe cart identity has an explicit contract
  rather than raw-cookie keying.

UX flow:
1. Eligible private request repeats -> it may reuse only that authorized
   principal/context's coherent representation.
2. Identity or permission changes -> old private output is not returned and the
   request renders current authorized state.

#### Generation advances travel with the write, not with the process

The RenderCache write side SHALL open in every process configured with
RenderCache enabled, whether or not that process serves HTTP, so that a write
made by a queue worker, a scheduled task, or a console command advances the
same generations the same write advances in the serving process. A persisted
permission generation advanced from such a process SHALL take effect, so that a
permission or membership change invalidates previously permitted private output
wherever the change was made. A process with RenderCache disabled by
configuration SHALL open nothing and SHALL write nothing to the ledger. Each of
these properties SHALL be proven by a test that performs the write outside the
served application and observes the next lookup rebuild, and the
honest-boundary statement and the manual statement that describe the present
limit SHALL be removed rather than left as stale warnings.

### Server stitching

For a Composite representation, Suprnova shall compose reusable public document
segments with request-specific or identity-bound Live island output on the
server before sending the response. Public seed-backed islands that introduce no
request-specific bytes may remain inside a Complete representation. Every
result remains a complete canonical document; the browser shall not inject
private content into a cached shell as a hydration requirement.

Acceptance criteria:
- Composite entries contain typed integrity-protected stitch slots, never
  executable placeholders or private prior output.
- A Complete entry contains only final reusable bytes; any island embedded in
  those bytes uses public seed state and no principal-bound instance metadata.
- Each slot declares component/view identity, parameters permitted in the
  shared segment, ordering, and surrounding HTML integrity.
- Request-time island rendering rechecks current authorization and records its
  own dependencies.
- Slots accept only framework-typed rendered HTML/metadata whose untrusted
  interpolation already passed the view contract; assembly preserves structure
  without arbitrary string insertion or double-escaping trusted fragments.
- A slot failure follows explicit fail-document, safe fallback, or omit behavior
  and cannot expose another principal's segment.
- Per-response CSP nonces, CSRF data, and other request-specific metadata are
  generated at assembly time where required.
- Final status, headers, `Content-Length`, and HTTP validator are derived after
  all Composite slots succeed and never reuse the graph's structural validator.

UX flow:
1. Application user requests a mostly public personalized page -> Suprnova
   reuses its Composite graph, renders authorized private islands, and sends one
   final complete document.
2. Private island render fails or is forbidden -> only its declared safe outcome
   appears; cached public output remains uncontaminated.

### Segment boundaries and composition safety

Cacheable segments shall have explicit structural ownership and dependency
metadata. Segmentation shall not permit arbitrary string splicing that breaks
HTML, island identity, security headers, or nested cache validity.

Acceptance criteria:
- Segment boundaries are generated or parsed through framework-owned typed
  markers.
- Nested segments have acyclic ownership and bounded depth/count.
- Segment identity and version participate in the enclosing representation.
- A child segment cannot relax the parent's privacy or security policy.
- Duplicate island or DOM identities introduced during stitching are detected.
- Assembly has deterministic output for equivalent inputs.

UX flow:
1. Route declares reusable and private boundaries -> build/check tooling proves
   structural composition where possible.
2. Assembly detects an invalid boundary -> it does not publish the malformed
   composite and follows render error policy.

#### Nested cached segments

A cached segment MAY contain cached segments. An inner cached segment SHALL
have an identity and a stored version that no including document owns, so
one stored copy is included from several documents and invalidated once
rather than once per including route: it is named, not recursed into. The
graph's segment list gains a recursive variant, `Segment::Nested { key:
RenderKey, version: u64, assembled_len: u32 }`, that carries the three
typed facts needed to reason about the inner segment without fetching it.
Carrying `assembled_len` in the naming variant preserves the safety property
assembly already relies on: the exact assembled length of a nested graph
SHALL be computable from the typed facts of every level before any byte is
copied, so the body bound is enforced before allocation exactly as it is
for a flat graph; a byte-recursive inner graph would not know its own length
until fetched and walked, which would destroy that property. The named facts
are a claim rather than a trust anchor: on resolution the fetched entry's
actual version and length SHALL be compared against what the graph named,
and a mismatch SHALL resolve through the declared policy below rather than
a silent substitution.

Ownership SHALL be acyclic and bounded in depth by `MAX_NESTING_DEPTH`,
initially 3, where depth is the length of the ownership chain and an unnested
composite is depth 1; the existing `MAX_SEGMENTS`, 193, continues to bound
the segments of each individual graph, and `MAX_NESTED_SEGMENTS`, initially
16, separately bounds how many of one graph's segments MAY be `Nested`, so
one document cannot fan out into hundreds of store reads. Acyclicity is
enforced by construction and checked at assembly: the assembler SHALL carry
the chain of ancestor keys as it descends, and resolving a segment whose
key already appears in that chain SHALL be a cycle failure rather than a
recursion; the depth bound alone is not sufficient, because it would still
terminate a cycle but report the wrong cause.

A failure inside an inner segment SHALL resolve through the same closed set an
island slot uses today, with no additions: `SlotFailurePolicy::{FailDocument,
Omit, Fallback { html }}`. An inner segment that cannot be fetched, whose
version or length does not match what the graph named, that exceeds the
depth bound, that forms a cycle, or that fails reauthorization, SHALL resolve
through the policy its including graph declared for it; a `FailDocument`
outcome SHALL abandon assembly for the whole document and fall through to
the uncached handler, exactly as it does for a slot today.

Privacy SHALL compose by narrowing only, enforced at publish rather than at
assembly: building a composite that names an inner segment whose representation
class is wider, or whose freshness window is longer, than the including
document's SHALL be refused at publish, reusing `RepresentationClass::narrowest`
as the existing comparison, so a bad composition can never be stored and the
hit path has one less way to fail.

Every inner segment SHALL be reauthorized per request and never cached,
reusing the slot mechanism: identity match against what the entry stored,
then `live.validate_request_context`, inside a slot scope. An inner segment
that is provably identity-free, meaning it declares no identity binding at
all, SHALL skip reauthorization; that is the contract's only escape from
this rule.

Telemetry SHALL distinguish an inner segment's outcomes from an island slot's
under one closed, low-cardinality metric, `suprnova.render_cache.stitch.nested`,
whose `outcome` attribute takes exactly one value from the closed set
`resolved`, `omitted`, `fallback`, `failed`, `depth_exceeded`, `cycle`,
and `version_mismatch`; no label carries a key, a route name, or an identity
digest. The framework SHALL offer one typed way to declare an inner cached
segment and its policy, and a declaration naming a segment the running build
no longer has SHALL fail that segment rather than substitute another. The
conformance corpus SHALL carry a nested case whose unknown depth or unknown
segment kind is rejected rather than ignored. Until iteration 006 delivers
this, server stitching caches exactly one level: every slot is re-rendered
on every hit, and a slot's island cannot itself be a shell with slots of
its own.

### Privacy and variance verification

Testing and diagnostics shall make cache-leak scenarios first-class. The
framework shall support assertions over classification, key dimensions,
stitching, and identity changes without revealing cached private bodies.

Acceptance criteria:
- Tests can render the same route as multiple principals, tenants, locales,
  sessions, and permission versions and assert separation.
- Diagnostics explain which observation caused classification or bypass.
- Property tests ensure unrecognized context dimensions cannot join public
  output silently.
- Cache metadata inspection redacts sensitive key material and body content.
- A production-safe audit event records prevented public/private contamination.

UX flow:
1. Application developer tests personalization -> harness proves variants and
   stitched slots do not cross identities.
2. Unsafe policy is detected -> checking/test fails before deployment with the
   observed source and recommended safe class.

#### Declined lookups record a reason

The `declined` lookup outcome SHALL carry a `reason` attribute whose values are
a closed enumeration fixed at compile time, with no request-derived text in it.
Every branch that declines a store or a serve SHALL map to exactly one reason,
and adding a decline branch without a reason SHALL fail to compile rather than
fall back to an unattributed default. The session-value decline, the
per-principal-gate decline, and the undeclared-locale decline SHALL each be
distinguishable from one another and from an ordinary ineligible response. The
reason set SHALL stay bounded under the closed low-cardinality label rule,
SHALL be documented beside `outcome` in the operations chapter and its mirrors,
and SHALL be asserted by the operations suite rather than only described in
prose.

## Acceptance criteria

- Every cached byte-affecting dimension is normalized and represented safely.
- Observed private state can only preserve or reduce sharing, never increase it.
- Private keys contain no reusable secrets and react to authorization change.
- Server stitching produces one complete authorized document without client
  injection.
- Tests exercise multi-principal and multi-tenant leak prevention.

## Decisions and revisions

- 2026-09-09 -- Recorded the nested cached segment mechanism ahead of code:
  naming through a recursive `Segment::Nested { key: RenderKey, version: u64,
  assembled_len: u32 }` variant rather than a byte-recursive graph,
  `MAX_NESTING_DEPTH` (3) and `MAX_NESTED_SEGMENTS` (16) as the new bounds
  beside the existing `MAX_SEGMENTS`, ancestor-chain cycle detection at assembly
  distinguished from a plain depth-exceeded outcome, publish-time refusal
  reserved for privacy narrowing, per-request reauthorization reusing the slot
  mechanism with an identity-free escape, and the closed
  `suprnova.render_cache.stitch.nested` telemetry outcome set, recorded under
  Segment boundaries and composition safety.
- 2026-09-09 -- Delivered the declined-lookup reason set: `LookupDeclineReason`
  (32 variants, `framework/src/render_cache/decline.rs`) types the `reason`
  attribute `LookupOutcome::record` emits beside `outcome="declined"`,
  computed at the branch that declines rather than reconstructed
  afterward; adding a decline branch without a reason, or a reason
  without a label, fails to compile. Every render-path invariant this
  wired through now fails safely rather than panicking: the four decline
  sites that used to assert or panic on a violated invariant now
  `debug_assert!` and decline in release, and `run_render` gained an
  error channel, `RenderRequestLost`, for the one failure `lead_render`
  cannot degrade to an uncached render because the request itself is
  already gone, including a transaction that fails at COMMIT after its
  closure already took the request; `lead_render` answers it with a
  controlled 500 and releases the lease so the route is not left fenced.
  `framework/src/render_cache/telemetry.rs` exposes
  `decline_reason_labels_for_test` so the operations-chapter
  documentation test can enumerate the closed set, recorded under Privacy
  and variance verification.
- 2026-09-08 -- Delivered the promoted authorization and RBAC requirements.
  Each authorization evaluation runs inside a consult window; principal
  material recorded inside it, or nothing the recording can resolve, SHALL
  require the `Principal` dimension, and tenant material alone SHALL require
  `Tenant` through the distinct `AuthorizationTenantRead` classification
  reason. The framework's RBAC reads name the five tables they read -
  `roles`, `permissions`, `role_permissions`, `model_roles`, and
  `model_permissions` - through crate-private observing statement helpers,
  so an RBAC-gated route is observed precisely instead of declining;
  application raw SQL keeps its documented boundary unchanged. A global
  scope declares whether its filter is constant or per-request, and a
  per-request evaluation that records no resolvable read SHALL narrow the
  render to `Uncacheable` and name the scope. The write side opens in every
  process whose configuration enables RenderCache and whose database holds
  the migration, probed at most once and never inside a caller's
  transaction.
- 2026-09-08 -- Promoted `authorization-reads-record-consulted-identity.md`
  from `iterations/next/` into iteration 006: an authorization decision
  records the identity axis and the concrete identity it consulted, recorded
  under Automatic privacy classification.
- 2026-09-08 -- Promoted `global-scope-tenant-observation.md` from
  `iterations/next/` into iteration 006: global-scope evaluation that reads a
  tenant records it as comparable material, recorded under Explicit variance
  model.
- 2026-09-08 -- Promoted `declined-lookups-record-a-reason.md` from
  `iterations/next/` into iteration 006: the `declined` lookup outcome carries
  a closed compile-time `reason`, recorded under Privacy and variance
  verification.
- 2026-09-08 -- Promoted `nested-cached-segments.md` from `iterations/next/`
  into iteration 006: a cached segment MAY contain cached segments under
  bounded, composing, reauthorized rules whose mechanism is recorded here
  before code, added under Segment boundaries and composition safety.
- 2026-09-08 -- Promoted `write-side-outside-the-serving-process.md` from
  `iterations/next/` into iteration 006: the write side opens in every
  RenderCache-enabled process so generation advances travel with the write,
  recorded under Private representation keys beside the permission-generation
  rule it repairs.
- 2026-09-08 -- Promoted `media-and-encoding-negotiation.md` from
  `iterations/next/` into iteration 006: `Media` and `Encoding` partition by
  the negotiated value over a closed declared set, recorded under Explicit
  variance model with the present constants named as the behavior it
  replaces.
- 2026-08-21 -- Adopted public, stitched, private, and uncacheable classes with
  automatic safety downgrades.
- 2026-08-21 -- Private personalization is composed on the server; rejected a
  cached public shell that requires browser injection to reveal initial content.
- 2026-08-21 -- Public seed-backed islands may remain in Complete immutable
  representations. Identity-bound or request-specific output makes the entry
  Composite and final HTTP metadata is computed only after assembly.
