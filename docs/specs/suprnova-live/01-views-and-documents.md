# Suprnova Live -- 01 Views and Documents

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the route-to-HTML contract: canonical documents, Suprnova's
view abstraction, external template authoring, render contexts, and the
server-rendered mounting boundary for Live islands. It depends on Suprnova's
router and HTTP response facilities and feeds component composition, the browser
runtime, document navigation, and RenderCache. Component behavior, snapshot
contents, and cache reuse decisions belong to their neighboring specs.

## Capabilities

### Canonical document responses

A real Suprnova route shall be able to return a complete HTML document whose
initial content is meaningful and exposed before browser enhancement. Live
shall not replace route authority with an SPA shell, JSON page protocol, or
partial-navigation transport.

Acceptance criteria:
- A normal GET returns a valid complete HTML representation with an appropriate
  status and content type.
- The representation exposes its initial content to clients that execute no
  JavaScript.
- The same route remains addressable through ordinary links, redirects,
  refresh, history traversal, `curl`, and non-browser HTTP clients.
- Live runtime metadata does not become the only source of application content.
- HEAD and conditional-request behavior can describe the same representation
  without requiring template execution when another subsystem has a valid
  stored representation.

UX flow:
1. Application user requests a real URL -> Suprnova returns the canonical HTML
   document or its normal HTTP error surface.
2. Browser enhancement succeeds or fails -> the initial document content
   remains exposed; only Live-dependent interaction changes.

### Suprnova view contract

Suprnova shall expose application rendering through a framework-owned view
contract rather than leaking a template engine throughout handlers and Live
components. Askama shall be the normative external-template authoring and
checked-template substrate for Suprnova Live. The stable `suprnova::view`
boundary shall isolate handler and component APIs from engine internals, but a
future template engine shall supply its own checker adapter and pass the same
view-contract conformance suite rather than claiming compatibility through an
unchecked generic renderer.

Acceptance criteria:
- Handlers and Live components render through stable Suprnova view APIs.
- External HTML templates support escaped interpolation, conditions, loops,
  reusable partials or includes, and layout composition.
- HTML escaping is safe by default; deliberate trusted markup requires an
  explicit auditable operation.
- Template lookup, compilation, and error reporting identify the originating
  template and source location where available.
- The Live view checker consumes Askama-compatible grammar and source structure
  rather than approximating template behavior with an unrelated parser.
- Template syntax passes unknown `live:` and `data-controller` attributes
  through as ordinary HTML.
- Application business and authorization logic are not required to live in
  templates.

UX flow:
1. Application developer selects a view from a route or component -> Suprnova
   supplies typed render data through the view contract.
2. Template compilation or rendering fails -> the developer receives a
   source-oriented diagnostic and no partial successful document is claimed.

### Render context and response metadata

Every document and island render shall execute with a defined context carrying
the request-scoped information permitted to affect output. Rendering shall
produce HTML plus the response metadata required by HTTP, Live, diagnostics,
and dependency collection without making templates responsible for transport
mechanics.

Acceptance criteria:
- The render context exposes explicit route, locale, request, identity, feature,
  asset, and dependency hooks subject to their owning security contracts.
- Access to context-derived values is observable by RenderCache dependency and
  variance collection where applicable.
- Rendering can set or contribute status, headers, content type, and declared
  assets through typed framework facilities.
- A render cannot silently emit headers after response commitment.
- Context APIs distinguish absent data from failed or unauthorized access.

UX flow:
1. Application developer renders context-aware content -> the framework records
   relevant dependencies and response metadata.
2. Required context is unavailable or forbidden -> rendering follows an
   explicit error or alternate-view path rather than substituting unsafe data.

### Live island mounting

A canonical document shall be able to contain one or more independently
identified, server-rendered Live islands. Mounting shall emit the initial island
HTML and either an instanced signed snapshot or an eligible public seed snapshot
without transferring ownership of the surrounding document. RenderCache may
retain a public seed inside a Complete representation, while identity-bound or
request-specific mounting metadata requires a Composite representation and
server assembly under the cache contracts.

Acceptance criteria:
- Each mounted island has a stable document-local identity and one explicit DOM
  boundary.
- Initial island HTML is server rendered and visible before runtime connection.
- Instanced snapshots and public seed snapshots are associated with the correct
  island slot without becoming executable inline application code.
- A public seed contains no principal-bound state or reusable Live instance
  identity; promotion to an instance belongs to the snapshot and protocol
  contracts.
- Multiple islands can mount in one document and connect independently.
- An island can render inside ordinary SSR markup without converting the route
  into Inertia or an SPA surface.
- Nested mounting preserves the ownership rules defined by the component and
  morphing specs.

UX flow:
1. Application developer mounts a component from a canonical view -> the
   server emits its initial island HTML and metadata.
2. Live runtime connects -> it adopts only the island boundary and leaves the
   surrounding document under normal browser ownership.

### Deterministic and failure-safe rendering

Given the same declared inputs and dependency generations, rendering shall be
stable enough for revision checks, content addressing, tests, and cache reuse.
A failed render shall not be published as successful representation output.

Acceptance criteria:
- Framework-generated ordering and metadata do not introduce avoidable
  nondeterminism.
- Rendering failures preserve the previously accepted island DOM during a Live
  interaction when safe.
- Initial document failures use ordinary Suprnova HTTP error handling.
- Partial buffers and snapshots from failed renders are not committed to
  RenderCache or sent as successful Live responses.
- Development diagnostics may be detailed while production responses avoid
  leaking templates, state, or secrets.

UX flow:
1. Rendering succeeds -> the complete output advances to its transport or cache
   owner.
2. Rendering fails -> the appropriate HTTP or Live recovery surface appears and
   the partial output is discarded.

## Iteration 002 implementation profile

Iteration 002 implements the host-neutral Suprnova view contract with Askama as
its normative checked substrate. It renders typed external templates into
deterministic document or island HTML plus typed status/header/content-type,
asset, mount, and diagnostic metadata. It also emits bounded initial island
boundaries with iteration-001 seed or instanced snapshots and detects duplicate
document-local identities, invalid nesting, partial-render failure, and unsafe
trusted-markup use.

The standalone profile accepts normalized route, locale, identity, feature, and
asset inputs only through Live host adapter contracts. It neither imports
Suprnova HTTP types nor registers routes in the active framework checkout. A
conformance adapter may prove canonical document and HEAD/conditional response
intent, but only the later atomic integration move may claim actual
`suprnova::view`, router, or `Response` integration.

## Acceptance criteria

- Real routes produce meaningful canonical documents without client rendering.
- The framework-owned view contract supports external HTML templates safely.
- Live islands mount as bounded SSR regions without taking over navigation.
- Render context use is observable to cache-safety machinery.
- Render failures cannot masquerade as valid documents, snapshots, or cache
  entries.

## Decisions and revisions

- 2026-08-21 -- Assigned the host-neutral Askama view contract, render metadata,
  and initial island emission to iteration 002. Actual Suprnova route/response
  adaptation remains part of the atomic integration move.
- 2026-08-21 -- Chose external HTML templates with Askama as the normative
  checked substrate behind `suprnova::view`. A future engine requires a checker
  adapter and conformance suite; rejected both leaking Askama through framework
  APIs and pretending arbitrary renderers are checked-compatible.
- 2026-08-21 -- Canonical HTML and real routes remain authoritative. Rejected
  SPA shells and partial-navigation protocols for Live.
- 2026-08-21 -- Public cache-safe islands may mount from reusable seed snapshots
  inside Complete representations; identity-bound mounting uses Composite
  assembly.
