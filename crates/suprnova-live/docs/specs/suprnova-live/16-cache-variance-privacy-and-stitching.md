# Suprnova Live -- 16 Cache Variance, Privacy, and Stitching

Status: Normative design specification
Last revised: 2026-08-21

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

## Acceptance criteria

- Every cached byte-affecting dimension is normalized and represented safely.
- Observed private state can only preserve or reduce sharing, never increase it.
- Private keys contain no reusable secrets and react to authorization change.
- Server stitching produces one complete authorized document without client
  injection.
- Tests exercise multi-principal and multi-tenant leak prevention.

## Decisions and revisions

- 2026-08-21 -- Adopted public, stitched, private, and uncacheable classes with
  automatic safety downgrades.
- 2026-08-21 -- Private personalization is composed on the server; rejected a
  cached public shell that requires browser injection to reveal initial content.
- 2026-08-21 -- Public seed-backed islands may remain in Complete immutable
  representations. Identity-bound or request-specific output makes the entry
  Composite and final HTTP metadata is computed only after assembly.
