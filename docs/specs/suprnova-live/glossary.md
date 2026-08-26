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

**Live host adapter contract**:
The internal framework-facing boundary through which the host supplies
normalized request facts, verified security context, sessions, transactions,
application services, and typed HTTP response intent to the host-neutral Live
kernel. A standalone conformance or test adapter proves the kernel contract but
is not actual Suprnova integration.
_Avoid_: third-party extension API, mock integration, parallel web framework, Suprnova facade

**Trusted Live request context**:
An internal capability constructed by a conforming Live host adapter only after
the current request passes the host's required origin, CSRF, session, principal,
tenant, proxy, and middleware checks, expressed as typed dispositions together
with current scope and mount-catalog facts. It is neither browser-constructible
nor a substitute for the component/action's current authorization decision.
_Avoid_: signed snapshot, client context, authorization proof, test fixture in production

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
ordering, Live instance-ledger record after identity-bound mount or seed
promotion, and DOM morph boundary; server actions rerender only that island.
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
revision; the first action promotes it into a freshly mounted scoped instance and
may overlay only its verified public fields.
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
instance's initial authority creation, base/successor revision, idempotency
identity, and accepted outcome metadata. It stores no persistent component
object and may use memory, the application database, or a conforming key/value
cache by deployment tier.
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

**Runtime feature artifact**:
A deterministic ESM or classic-script production file for a declared optional
Live capability, selected by trusted rendered roles through the typed asset
manifest and registered into the one core runtime. Iteration 004 defines
separate upload and asynchronous artifact pairs so other Live pages do not pay
their transfer cost.
_Avoid_: plugin URL from HTML, second runtime, application bundle requirement, arbitrary module

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

### Uploads and asynchronous updates

**Temporary upload**:
Revisioned, expiring, quarantined server/provider state that receives and
verifies one selected file before an authorized application action may finalize
it. Its lifecycle is independent of component snapshot revision.
_Avoid_: permanent file, model value, browser File, public object

**Upload handle**:
An opaque bounded identifier for one temporary upload that may travel as a typed
component/action value but grants no transfer or finalization authority. Every
use is reauthorized for current principal/session, tenant, component, field,
policy, and upload state.
_Avoid_: upload token, file path, storage key, authorization proof

**Transfer grant**:
A separate short-lived secret authorizing only declared bounded upload
control/data operations for one temporary upload. It remains in current-document
runtime memory and never enters snapshots, HTML, URLs, history, action/model
envelopes, logs, traces, or diagnostics.
_Avoid_: upload handle, signed snapshot, session token, resumable public URL

**Upload proposal authority**:
The core-owned narrow capability that admits only `null`, one canonical upload
handle, or a bounded ordered handle list for the exact declared upload field of
the current island, then writes it through the ordinary typed model batch. It is
not a generic model, snapshot, action, or feature-authority mutation port.
_Avoid_: generic feature write, transfer grant proposal, automatic upload action

**Upload file identity**:
A bounded current-document comparison tuple of sanitized display name, byte
size, browser MIME claim, and last-modified value used to match a user-held
`File` during explicit reacquisition. It is never trusted storage identity,
content validation, path authority, or proof of ownership.
_Avoid_: quarantine object, authoritative MIME, browser path, upload authority

**Quarantine store**:
A host-supplied asynchronous byte-I/O capability for exclusively creating,
writing, syncing, reading, and removing non-public temporary upload objects. It
keeps executor and filesystem choice outside the engine while server-generated
identity, hashing, lifecycle, and policy remain Live-owned.
_Avoid_: public storage, client path, blocking engine filesystem, upload ledger

**Quarantine object**:
A server-random, fixed-grammar storage identity used only behind the quarantine
store boundary. It is neither a client filename or path nor an upload handle,
and it exposes no public or serving location.
_Avoid_: browser path, original filename, public object key, upload handle

**Transfer checkpoint**:
Bounded non-path provider state that records an opaque quarantine object,
sequential accepted chunks, byte count, and transfer identity so a host can
persist and reconstruct interrupted work without trusting browser metadata.
_Avoid_: local file path, component snapshot, transfer grant, ambient browser resume

**Direct transfer instruction**:
A short-lived provider-origin-, method-, part-, header-, and byte-bound
capability that permits the browser to send one part directly to configured
storage while Live retains handle and lifecycle authority. It may contain
provider credentials and is therefore redacted from diagnostics.
_Avoid_: transfer grant, vendor upload API, public upload URL, storage authority

**Direct part reference**:
An opaque non-authoritative identity binding a provider part report to one
temporary upload and exact byte range. The adapter verifies provider state for
that binding; the browser reference itself is never completion or integrity
evidence.
_Avoid_: provider receipt as authority, checksum claim, upload handle, transfer grant

**Accepted upload type**:
A digest-significant field-policy contract pairing one canonical MIME type with
its permitted filename extensions. Matching requires a built-in or trusted
application-classified authoritative type; browser MIME and filename claims are
never classification evidence.
_Avoid_: browser accept value, MIME claim, filename suffix as proof

**Application-classified upload type**:
A canonical MIME type returned by a trusted application classifier after it
inspects authoritative quarantined content the bounded built-in classifier does
not recognize. It is explicit trusted validation output, not a browser claim.
_Avoid_: client content type, accepted upload type, inferred filename type

**Validation evidence**:
An immutable record binding authoritative size, checksum, content
classification, media facts, scope, policy digest, and the exact Ready revision
that finalization must reauthorize.
_Avoid_: browser metadata, provider receipt, scan result alone, durable file

**Finalize token**:
An opaque host-finalizer identity for one prepared durable-storage operation,
used only for idempotent commit, compensation, and reconciliation. It is never a
browser capability or public storage location.
_Avoid_: transfer grant, upload handle, public URL, database transaction

**Upload finalizer**:
The host capability that prepares and commits durable upload storage and exposes
explicit compensation and reconciliation after partial failure. It does not
claim distributed atomicity or exactly-once external effects.
_Avoid_: upload provider, database transaction, exactly-once file mover

**Cleanup lease**:
A short ledger-owned, revision-fenced claim granting one trusted worker a
bounded interval to idempotently reclaim a terminal temporary upload. Expired
active authority becomes `Expired` atomically before the lease is issued;
`Finalizing` and `Finalized` authority is never eligible.
_Avoid_: transfer grant, browser callback, storage lock, permanent ownership

**Cleanup orphan**:
A terminal temporary upload whose failed cleanup attempts crossed the configured
finite operations threshold. The marker raises operational visibility but never
abandons the upload; capped idempotent reconciliation continues.
_Avoid_: permanent leak, dropped retry, browser-owned cleanup, failed upload state

**Subscription descriptor**:
A signed, expiring, non-secret server-issued integrity authority memo for one
permitted asynchronous subscription, including registered stream identity,
capabilities, resolved topics, full typed-event contracts,
authorization-context memo, authoritative baseline epoch/sequence, reconnect
policy, and bounded hybrid fallback. It is not proof of current authorization;
the current component contract and stream registration are re-resolved at each
boundary, and only the host continuity authority supplies the issuance
baseline. Transport credentials remain separately secret, descriptor-scoped,
operation-scoped, expiring, uniquely minted, and atomically single-use when
required. One host-owned transaction consumes Connect and persists Renew, or
consumes Renew and persists Connect, after all non-mutating checks succeed.
_Avoid_: channel name from HTML, WebSocket URL authority, global event bus, action dispatch token

**Subscription binding**:
A compact non-secret digest of the exact signed subscription-descriptor wire,
including signing key ID and signature. It distinguishes equal claims signed
under overlapping keys and is compared at membership/control boundaries without
making the descriptor or credential loggable.
_Avoid_: claims hash, authorization memo, transport credential, subscription ID

**Subscription ID**:
A bounded opaque non-secret identity for routing one authorized logical
subscription inside a compatible multiplexed document transport. It must match
active membership before sequence observation or dispatch and grants no
authorization by itself.
_Avoid_: transport credential, channel authority, document transport handle, session ID

**Active async membership context**:
The cloneable sealed framework value produced when a Task 2 connect-authorized
subscription and one host-owned membership/current-registry snapshot agree on
exact logical subscription ID, registered stream, full event contracts,
declared presentation signals, and the verified signed descriptor baseline. It
is a static codec and sequence-scope contract; it is not fresh membership or
dispatch authority.
_Avoid_: current membership proof, caller-supplied allowlist, transport handle, dispatch token

**Active async membership guard**:
A non-cloneable one-use framework capability issued only after one envelope
freshly passes exclusive descriptor expiry, active host membership, exact
subscription/stream scope, current full event and presentation-signal
contracts, and closed-payload validation. The sequence machine consumes it
while rechecking exclusive descriptor expiry and owning registered dispatch and
position commit; it is not physical transport membership or a caller-markable
commit token.
_Avoid_: async envelope context, reusable membership token, transport handle, success flag

**Transport membership authority**:
The fresh host-owned decision consumed at every external SSE/WebSocket logical
membership add or remove. The host re-resolves the current component/identity
authorization memo, active subscription, stream, resolved topics, full event
contracts, canonical registered modes, and exact document/control policy; the
framework independently compares those facts to the descriptor-bound request
and rechecks exclusive expiry. Add performs the check before source work and
again after asynchronous source establishment immediately before commit. It is
not the subscription descriptor, browser control record, document transport
handle, physical adapter kind, or a reusable caller-constructible token.
_Avoid_: authorized transport token, document-handle authority, cached membership check, mode hint

**Document authorization scope**:
The compact collision-resistant physical-transport sharing key derived only
from trusted aggregate scope, principal, session, tenant, and explicit host
transport-policy identity. It deliberately excludes component name and contract;
each logical membership still requires its own component authorization memo and
exact subscription binding.
_Avoid_: component memo, browser scope string, document handle, origin, transport kind

**Document control generation**:
The monotonic document-owned version captured with an exact private document-
owner identity by a prepared external membership control and compared again at
its pre-source gate or final synchronous commit.
Any intervening membership or retirement mutation makes the one-use control
stale. It is internal concurrency metadata, not a browser nonce, revision,
subscription sequence, or authorization source.
_Avoid_: component revision, document handle, browser generation, sequence position

**Pending transport control**:
A one-use, hard-bounded server capability carrying an exact document snapshot
through fresh asynchronous membership authorization or source establishment
without borrowing the document transport. It owns its permit and, after a
source opens, once-only cleanup until synchronous commit accepts ownership.
_Avoid_: reusable admission token, async mutable document lock, browser control authority, detached task

**Transport retirement lane**:
The document-owned hard-bounded collection of logical sessions already detached
from active routing but still requiring idempotent provider cleanup. Persistent
fair close polling retains ownership across pending calls, failures, cancellation,
completion, authorization expiry, and shutdown without spawning detached work.
Each entry retains its exact subscription ID and signed-descriptor binding as a
re-admission and capacity fence until cleanup succeeds.
_Avoid_: active membership, background leak, unbounded cleanup queue, Task 5 backpressure buffer

**Asynchronous event envelope**:
The independently versioned canonical bounded message that binds an active
subscription ID, registered stream, monotonic position, and one closed
registered presentation or lifecycle payload. It is not a Live action/morph
response and cannot carry HTML, snapshots, arbitrary actions, or executable
effects.
_Avoid_: Live protocol v3, streamed DOM patch, event bus message, arbitrary push payload

**Sequence authority**:
The per-logical-subscription state machine initialized only from the signed
baseline retained by its sealed active async membership context. It applies only
an exact same-epoch successor after registered dispatch succeeds, ignores
duplicates and older epochs, degrades on gaps or new epochs, and validates every
replay transition used to recover either its own degraded high-water or an exact
bounded-delivery pressure high-water while the sequence lane remained current.
No pressure tracker owns a parallel sequence counter.
_Avoid_: last message wins, transport ordering, reconnect success, client timestamp

**Replay transcript**:
A bounded ordered set of freshly membership- and registry-admitted envelopes
covering every same-scope, same-epoch position from the sequence authority's next
required successor through at least the exact recorded gap or pressure-loss
high-water. The existing sequence machine validates and commits the transcript
even when pressure, rather than a sequence observation, created the obligation.
Claimed `from`/`through` positions, empty evidence, duplicate positions, gaps,
and cross-scope or cross-epoch messages are not proof.
_Avoid_: boolean continuity flag, claimed range, reconnect success, transport ordering

**Async backpressure buffer**:
The server delivery admission policy wrapped around the shared resource owner,
single document-owned bounded queue, permit pool, and cancellation flag. It
accepts only sealed authorized async buffer entries, applies hard aggregate
item, byte, payload, replay, trusted fanout, policy fanout, and delivery-work
limits, and may coalesce only an exact contiguous tail with the same descriptor
binding, document/component scope, subscription, stream, epoch, registered
identity, and presentation schema contract. It is neither the transport
retirement lane nor a second sequence or lifecycle authority.
_Avoid_: private async queue, event bus, replay authority, transport retirement lane

**Pressure continuity tracker**:
The bounded redacted set of unresolved delivery-loss causes keyed by exact
logical subscription binding and document/component scope. It records authorized
lost high-water facts but owns no sequence counter; complete replay clears only
the exact causes it covers, while authenticated explicit retirement discharges
only that retired binding's obligation. Its four finite cause classes are capped
at four times the document membership ceiling; saturation remains degraded until
aggregate retirement rather than forgetting an unidentified loss.
_Avoid_: document-wide degraded boolean, global recovery flag, sequence authority, telemetry label map

**Authorized async buffer entry**:
A private non-forgeable one-use delivery value created only inside the
document-owned admission operation after the exact active Task 4 membership and
Task 3 current registry authority validate descriptor expiry/binding, document
and component scope, revocation, the complete payload contract, and host-resolved
event recipients/target-set scope. It cannot escape as an application-held
seal or later buffer offer. It owns the same membership guard later consumed by
the exact logical binding's existing sequence machine.
_Avoid_: envelope plus fanout count, browser delivery claim, reusable membership proof

**Async delivery lease**:
The private non-cloneable RAII value created when the document owner removes one
buffer entry for immediate closed dispatch. It retains the exact authorized
entry, shared permit and cancellation state, membership binding/scope, and
pressure-continuity tracker until the document revalidates current authority and
invokes the registered Task 3 dispatcher. An unresolved drop, denial,
cancellation, or dispatcher failure releases the permit and degrades continuity;
callers cannot mark it successful.
_Avoid_: public pop result, caller acknowledgment token, detached delivery future

**Resolved async delivery capability**:
The non-cloneable private-construction value consumed by registered dispatch,
binding one freshly accepted envelope to the host-resolved target-scope digest,
recipient count, and deployment fanout limit. It carries delivery proof rather
than letting a dispatcher or browser substitute recipients after admission.
_Avoid_: caller fanout count, target list hint, reusable dispatch token, raw envelope

**Bounded document transport session**:
The cohesive server owner that composes one Task 4 round-robin physical document
transport with one aggregate Task 5 queue and shared delivery permits. It pulls
at most one provider item per pump, owns no hidden per-island ingress buffer, and
owns exactly one Task 3 sequence lane per exact logical subscription binding so
the caller cannot inject another sequence authority. Its finite redacted pressure
tracker retains unresolved causes by exact binding and scope; recovery clears
only covered causes for that membership. A detached terminal predecessor retains
its sole lane as an exact/rotated same-ID re-admission fence until delivery drains.
_Avoid_: per-island backpressure queue, caller-supplied sequence machine, second event bus, document work arbiter

**Buffer disposition**:
The closed outcome of one server delivery admission: `Queued`, `Coalesced`,
`Degraded`, or `Closed(code)`. Coalesced means exact replaceable work was
absorbed by the current tail: an equal or older redundant request retains that
tail without creating a continuity loss, while exact-successor replacement
marks the affected membership degraded. `Degraded` always names an existing
exact recovery obligation rather than malformed replay input.
_Avoid_: success boolean, best-effort drop, currentness proof, transport status

**Credential rotation uncertainty**:
A subscription credential rotation that committed predecessor consumption and
successor persistence but lost the successor response. The predecessor cannot
replay and the undisclosed successor cannot be recovered; the client obtains a
freshly issued subscription and credential. This is not an idempotent outcome
record or permission to retry the predecessor.
_Avoid_: retryable provider failure, reusable credential, Task 3 sequence result

**Document transport**:
One physical SSE or WebSocket connection owned by a browser document for a
compatible origin, transport kind, and document authorization scope. Compatible
logical memberships additionally have one exact current credential contract and
a nonempty intersection of reconnect bounds; the grouping key never elects the
first island's credential or retry policy as document authority. It multiplexes
bounded subscription identities; it is not one connection per Live island.
_Avoid_: island socket, global shared socket, subscription authority, event bus

**Stream continuity**:
Proof that every required typed event after an authoritative baseline has been
accounted for through an unbroken sequence or complete validated replay
transcript. A reconnect without that proof remains degraded until authoritative
host refresh establishes a new baseline that covers observed high-water.
_Avoid_: socket connected, eventual freshness, best-effort ordering, last message wins

**Presentation-only stream update**:
A registered typed asynchronous event that may change a declared local signal
but cannot write component, revision, authorization, accepted-outcome, or domain
state.
_Avoid_: streamed action, client mutation, DOM patch, authoritative event

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
- An identity-bound initial mount creates ledger authority before its output can
  be published. A public seed snapshot instead promotes on first action; the
  Live instance ledger then arbitrates either instance's revisions and accepted
  outcomes.
- A Live action changes server-authoritative state and returns new island HTML;
  a morph reconciles that HTML within the island boundary.
- A Live host adapter establishes the trusted Live request context before the
  host-neutral endpoint service can promote, hydrate, authorize, dispatch, or
  render; standalone conformance adapters prove this boundary without becoming
  Suprnova integration.
- Live directives connect template markup to local behavior or the Live
  protocol; accepted responses may request only registered browser effects.
- A selected browser file creates a temporary upload. The upload handle may
  enter typed Live state, while its transfer grant stays outside all snapshots
  and markup. Authoritative validation stores exact Ready-revision evidence;
  only an authorized action may invoke the upload finalizer or abandon the upload.
- A subscription descriptor authorizes one asynchronous subscription contract;
  compatible subscriptions share a document transport, while stream continuity
  or authoritative refresh determines whether material freshness can be called
  current.
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
  Components may opt into protocol-v2 `refresh_on_promote`, which accepts fresh
  render and does not execute the original first-action intent.
- **Complete** -- the methodology term means nothing agreed remains, while a
  **Complete representation** is a directly sendable cache-entry type; the
  compound cache term must never be used to claim project completion.
