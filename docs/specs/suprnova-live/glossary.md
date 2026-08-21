# Glossary

Canonical language for this project. Definitions are written in this
project's sense; when a term here conflicts with a model's prior, this file
wins. Entries are revised in place as understanding deepens -- never left
stale. A term enters only after both parties have confirmed the meaning.

Entry format: bold term, tight one-or-two-sentence definition of what the
term IS, then an Avoid line listing rejected synonyms.

## Working vocabulary

**Done**:
Implemented and verified. Code without its verification is not done.
_Avoid_: mostly done, done pending tests

**Complete**:
Nothing agreed remains. Work is complete or it is not.
_Avoid_: essentially complete, complete except

**Defer**:
A developer verdict that moves agreed work out of scope. Models surface;
only the developer defers.
_Avoid_: out of scope (as a model verdict), later, phase 2

**MVP**:
Not used in this methodology. The spec is the scope; there is no smaller
version of done.
_Avoid_: slim version, first cut, v1 scope

**Refactor**:
A behavior-preserving change. If behavior changes, it is a feature or a fix.
_Avoid_: rewrite (unless it truly is one), cleanup (when behavior changes)

**Spec**:
The agreed contract for a domain. Normative, not advisory.
_Avoid_: notes, suggestions

**Iteration**:
The current scope contract (iterations/NNN.md): what ships now, what
explicitly does not.
_Avoid_: sprint, phase

**Drift**:
Any divergence between an artifact and reality -- in code, specs, or
vocabulary.
_Avoid_: slippage, staleness

## Project terms

Added during Stage 1 of /new-project and whenever a new term earns its
place. Group under subheadings when natural clusters emerge.

### Documents and interaction

**Application developer**:
The person using Suprnova Live to build, verify, and operate a Suprnova
application. This is the consumer of Live's Rust, template, tooling, testing,
runtime, and component-library contracts.
_Avoid_: end user, framework user, unqualified user

**Application user**:
The person interacting with an application built by an application developer.
This person experiences canonical documents, Live islands, local interactions,
server actions, feedback, navigation, and recovery behavior rather than the
framework authoring APIs directly.
_Avoid_: developer, framework user, unqualified user

**Suprnova view contract**:
The framework-owned rendering interface through which routes and Live components
select external templates, provide render data, and receive HTML plus response
metadata. Askama is the normative checked Live authoring substrate behind this
interface; another engine requires its own checker adapter and conformance.
_Avoid_: Askama API as handler contract, unchecked generic renderer, inline HTML builder

**Canonical document**:
The complete HTML representation returned by a real route, with initial
content that is meaningful and exposed before browser enhancement. It may be
fully cached or assembled from cached public segments and private Live
islands; it is never merely a JavaScript bootstrap shell, but Live interaction
may require the Live browser runtime.
_Avoid_: SPA shell, hydration shell, fallback page

**Progressive enhancement**:
Suprnova routes and initial server-rendered content exist independently of
JavaScript, while Suprnova Live adds runtime-dependent interaction to that
HTML. It does not require action parity, generated fallback handlers, or
synthesized no-JavaScript execution paths.
_Avoid_: automatic fallback, no-JavaScript parity, duplicate transport

**Live island**:
An independently identified, server-rendered region inside a canonical
document that becomes interactive when the Live browser runtime connects. It
owns its component lifecycle, seed-or-instanced snapshot state, request
ordering, Live instance-ledger record after promotion, and DOM morph boundary;
server actions rerender only that island.
_Avoid_: SPA component, microfrontend, Topcoat shard, replaceable HTML fragment

**Local signal**:
Browser-owned, island-scoped UI state for behavior that requires neither server
authority nor server computation. It survives a morph while its keyed scope
survives, resets when that scope or document is removed, and must never hold
authoritative domain or security state.
_Avoid_: component state, snapshot state, global store, client authority

**Component state**:
The typed Rust state of a Live component carried across actions and renders. It
is client-carried by default inside a signed snapshot but remains
server-authoritative; the browser may propose changes only to fields explicitly
exposed for model binding. Transient model values are request-only and never
join dehydrated component state.
_Avoid_: local signal, DOM state, persistent server component, browser authority

**Model-bindable field**:
A component-state field explicitly declared to accept typed value proposals from
browser controls. Binding permission changes what the browser may propose, not
which side is authoritative.
_Avoid_: public field, mass-assignable state, trusted form value

**Transient model field**:
An explicitly bindable request-only field for sensitive or ephemeral input such
as a password or one-time code. Its typed value may be consumed by the current
action but is never dehydrated, cached, logged, or implicitly available to a
later request.
_Avoid_: snapshot secret, persistent password field, server echo, hidden durable state

**Signed snapshot**:
A server-issued, versioned and integrity-protected Live state envelope whose
contents are visible browser data rather than secret. The canonical forms are a
public seed snapshot before instance creation and an instanced snapshot after
mount or promotion.
_Avoid_: encrypted state, server session, authorization proof, trusted client payload

**Public seed snapshot**:
A reusable signed snapshot form embedded only in cache-safe public island HTML.
It binds public component/build, route, slot, parameters, state, issue age, and
advisory generation memo but contains no principal-bound data, instance ID, or
revision; the first action promotes it into a new scoped instance.
_Avoid_: anonymous instance, authorization token, cached private snapshot, permanent mount

**Instanced snapshot**:
A signed snapshot bound to one promoted or freshly mounted Live instance,
including its instance identity, base revision, bounded validity, state, and
lifecycle memo. Missing ledger authority requires fresh rendering rather than
reconstructing the instance from this browser-carried envelope.
_Avoid_: seed snapshot, persistent component object, instance authority, database row version

**Verified snapshot capability**:
An internal Rust value constructible only after a signed seed or instanced
snapshot passes canonical parsing, integrity, version, time, schema, and trusted
binding checks. Only this capability exposes typed hydration; possessing it does
not supply request authenticity, authorization, or ledger authority.
_Avoid_: parsed snapshot, authorization token, trusted browser state, hydration bypass

**Live instance ledger**:
The expiring provider-backed concurrency record that atomically arbitrates one
instance's base/successor revision, idempotency identity, and accepted outcome
metadata. It stores no persistent component object and may use memory, the
application database, or a conforming key/value cache by deployment tier.
_Avoid_: component session, sticky server object, generation ledger, domain transaction log

**Accepted Live outcome**:
The one committed protocol outcome permitted for an island base revision. A
rolled-back Rust method may run again, and external side effects require their
own idempotency/delivery contract; the term does not promise exactly-once method
invocation.
_Avoid_: exactly-once action, method invocation, external-effect guarantee, browser click

**Island revision**:
The monotonically advancing version claimed through the Live instance ledger
for one Live island, used to arbitrate accepted outcomes and reject stale work.
It is not a database-record version or a substitute for domain concurrency.
_Avoid_: row version, timestamp, global page version, authorization version

**Morph**:
The bounded reconciliation of an existing Live island's DOM with newly
server-rendered HTML while preserving keyed identity, focus, form state, local
signals, and controller continuity where the Live contract permits. It applies
to the island boundary, not the entire document.
_Avoid_: full-page rerender, virtual DOM, wholesale replacement, client rendering

**Live browser runtime**:
The Suprnova-owned JavaScript runtime that connects Live islands and implements
Live directives, local signals, model synchronization, action transport,
request scheduling, effects, and morphing. Stimulus complements it for custom
controllers but does not define the Live protocol.
_Avoid_: SPA runtime, Stimulus, Turbo, client renderer

**Live directive**:
A namespaced declarative `live:` HTML attribute whose registered name, value,
target, and modifiers are interpreted consistently by the view checker and Live
browser runtime. It is not an inline JavaScript expression.
_Avoid_: event-handler script, arbitrary expression, Stimulus action, wire directive

**Live protocol**:
The versioned request and response contract used by the Live browser runtime to
synchronize an island, invoke registered actions, and receive rendered HTML,
snapshots, errors, events, effects, or redirects.
_Avoid_: SPA page protocol, arbitrary RPC, application route, WebSocket foundation

**Trusted interaction spine**:
The complete iteration-001 foundation for canonical signed snapshots, public
seed promotion, instance-ledger revision authority, versioned Live envelopes,
safe recovery, and shared Rust/TypeScript conformance. It is a build-order
milestone, not a smaller product definition or claim that the full Live system
is adoption-ready.
_Avoid_: MVP, demo protocol, throwaway backend, reduced Live mode

**Live action**:
An explicitly registered, typed Rust method invoked through the Live protocol
to perform a server-authoritative interaction. Ordinary HTTP handlers may share
domain services with a Live action but remain separate transport entry points.
_Avoid_: arbitrary method call, controller route, synthesized no-JavaScript handler

**Browser effect**:
A named, registered, schema-validated data instruction applied by the Live
browser runtime after an accepted response at a protocol-defined phase.
_Avoid_: arbitrary JavaScript, eval payload, inline script, unregistered callback

**Document navigation**:
Normal browser navigation to a real Suprnova route that returns a complete
canonical document. It may be prefetched or visually transitioned without
becoming client routing or a partial-page navigation protocol.
_Avoid_: SPA navigation, client router, Turbo visit, wire-style navigation

**URL reflection**:
The replacement of the current same-route query URL with
`history.replaceState` after a Live state change. It creates no new history
entry or `popstate` action path; state needing Back/Forward steps uses document
navigation.
_Avoid_: client routing, pushState navigation, Live history stack, popstate refresh

### Render caching

**RenderCache**:
The optional framework-managed cache of Complete or Composite canonical HTTP
representations together with metadata needed to prove safe reuse. It is
distinct from the generic application cache; Live remains fully functional when
RenderCache is disabled or unavailable.
_Avoid_: generic cache, template cache, response TTL, static-site generator

**RenderCache entry**:
A typed versioned Complete or Composite representation plus the variance,
dependency, policy, format, integrity, and coherence metadata required to prove
safe reuse.
_Avoid_: serialized template context, ORM result cache, HTML string without proof

**Complete representation**:
A directly sendable RenderCache entry containing final immutable body bytes,
safe HTTP metadata, and a validator for exactly those bytes. It may embed public
seed-backed islands but contains no request-specific stitch slots or instanced
private state.
_Avoid_: complete project, cached shell, segment graph, preassembled private response

**Composite representation**:
A RenderCache entry containing an integrity-protected structural segment graph
and typed stitch slots that requires request-time server assembly. Final bytes,
recipient-specific metadata, `Content-Length`, and HTTP validators exist only
after successful assembly.
_Avoid_: finished response, arbitrary string template, client composition, public seed page

**Dependency collector**:
The request/task-scoped recorder active across the opted-in cacheable handler
and rendering pipeline that observes authoritative data, configuration,
identity, and version inputs used to produce a representation.
_Avoid_: manual cache-tag list, global query log, implicit process state

**Dependency generation**:
A monotonically advancing version for a data or configuration dependency. A
rendered entry records the generations it observed and becomes invalid when a
relevant current generation differs.
_Avoid_: query hash, cache tag, deletion list, TTL

**Generation ledger**:
The application-database authority for logically append-only committed
dependency generations and their authority epoch at every cache deployment
tier. Memory, Redis, Memcached, and hints may accelerate observation but never
replace its correctness role.
_Avoid_: pub-sub channel, Redis hint, local counter, eviction index

**Cache deployment tier**:
A provider configuration selecting RenderStore, LiveInstanceLedger,
RebuildCoordinator, and GenerationLedger implementations without changing
application behavior: Embedded, Database-coordinated, or Externally accelerated.
Tier 0 is complete rather than a reduced Live mode.
_Avoid_: feature tier, Redis requirement, degraded local mode, paid capability

**Cache variance**:
The explicit or automatically detected dimensions that distinguish cached
representations, such as route parameters, locale, principal, tenant,
permission version, and negotiated representation. Variance uses stable,
purpose-specific identifiers rather than raw session IDs or arbitrary cookies.
_Avoid_: cache busting, raw cookie key, global personalization key

**Server stitching**:
Per-request assembly of a Composite representation's shared segments and typed
slots with private or otherwise request-specific server output before the
response is sent. It preserves a complete server-rendered document without
contaminating stored public data.
_Avoid_: client injection, hydration, public personalization, SPA composition

**Stitch slot**:
A typed, integrity-protected structural position inside a Composite
representation where Suprnova inserts freshly authorized request-specific
server output during server stitching.
_Avoid_: arbitrary string placeholder, client mount point, cached private HTML

**Coherence policy**:
The explicit contract governing how RenderCache proves that a stored
representation is still valid and how much staleness, if any, it may serve.
The policy controls validation authority, local-cache leases, invalidation
behavior, and stale-while-revalidate eligibility.
_Avoid_: TTL-only correctness, eventual-consistency handwave, best-effort invalidation

**Singleflight rebuild**:
The fenced coordination that permits one accepted publication for a RenderCache
key and coherence epoch while other requests wait, receive policy-permitted
stale content, or bypass. Distributed failure may cause bounded duplicate
computation, never an unfenced stale publication.
_Avoid_: cache lock without fencing, global mutex, thundering herd

### Component library

**Theme token**:
A versioned semantic design value for roles such as color, typography, spacing,
radius, motion, density, or interaction state that drives official component
presentation across compatible themes.
_Avoid_: raw palette value, Tailwind utility class, component-specific hard-code

## Relationships

- A real route returns a canonical document through document navigation.
- A canonical document may come from a Complete representation or from server
  assembly of a Composite representation and can contain one or more Live
  islands.
- The Live browser runtime connects each Live island. Local signals remain in
  the browser, while component state crosses requests in a public seed or
  instanced signed snapshot.
- A public seed snapshot promotes on first action into a new scoped instance;
  the Live instance ledger then arbitrates its revisions and accepted outcomes.
- A Live action changes server-authoritative state and returns new island HTML;
  a morph reconciles that HTML within the island boundary.
- Live directives connect template markup to local behavior or the Live
  protocol; accepted responses may request only registered browser effects.
- Dependency generations, cache variance, and the coherence policy jointly
  determine whether a RenderCache representation remains valid.
- During opted-in rendering, the dependency collector records generations from
  the generation ledger, then a fresh post-render reread gates publication;
  invalid entries use fenced singleflight under policy.
- Server stitching resolves Composite stitch slots with freshly authorized
  request-specific output before the canonical document is sent.
- Cache deployment tiers change providers and topology, never application-facing
  Live or RenderCache correctness semantics.
- Document navigation replaces the document and resets document-scoped local
  signals; URL reflection changes only the current same-route query and does not
  convert Live into a SPA.

## Flagged ambiguities

Terms that collided and how they were resolved. Worked example from the
methodology's own history:

- **UX specification** -- collided between visual design language and
  interaction design; resolved: how the user interacts with the software
  (flows, surfaces, journeys). _Avoid_: UI design, design language
- **Canonical document usability** -- "functionally usable before browser
  enhancement" implied that Live actions needed synthesized no-JavaScript
  equivalents; resolved: initial content is meaningful and exposed without
  JavaScript, while Live interaction requires its browser runtime unless the
  application explicitly supplies ordinary Suprnova routes, forms, or links.
- **Progressive enhancement** -- collided with automatic no-JavaScript action
  parity; resolved: Suprnova and initial SSR content work without JavaScript,
  while Live interactions require the Live browser runtime and receive no
  synthesized alternate transport.
- **Stateful component** -- collided with a persistent server-resident object;
  resolved: Live presents a stateful programming model through signed snapshots
  while request execution remains stateless by default.
- **Live island versus shard** -- both describe server-rendered regions, but a
  Live island owns persistent component semantics, a signed snapshot, targeted
  action handling, and identity-preserving morphs rather than wholesale fragment
  replacement.
- **User** -- collided between the person building with Suprnova and the person
  using the resulting application; resolved: use **application developer** and
  **application user** respectively, never the unqualified term where the role
  matters.
- **Browser effect** -- collided with arbitrary server-returned JavaScript;
  resolved: an effect is registered behavior receiving validated data and runs
  only after an accepted response at a defined protocol phase.
- **Fresh versus valid cache entry** -- freshness is a policy time state, while
  validity requires successful coherence proof; a policy may serve known-stale
  content without mislabeling it as current.
- **At-most-once action** -- collided between method invocation and committed
  protocol outcome; resolved: only one committed Live outcome may be accepted
  per island base revision. A rolled-back method may run again, and external
  effects retain their own idempotency contract.
- **Seed freshness** -- collided between cache-generation freshness and safe
  action authority; resolved: public-seed generations are advisory by default,
  because every action must reload/reauthorize current authoritative data.
  Components may opt into `refresh_on_promote` for current mount-state UX.
- **Complete** -- the methodology term means nothing agreed remains, while a
  **Complete representation** is a directly sendable cache-entry type; the
  compound cache term must never be used to claim project completion.
