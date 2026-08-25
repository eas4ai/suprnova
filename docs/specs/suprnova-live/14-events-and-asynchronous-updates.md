# Suprnova Live -- 14 Events and Asynchronous Updates

Status: Normative design specification
Last revised: 2026-08-25

## Scope

This domain owns declared component/browser events, cross-island communication,
server-pushed refresh through host-provided broadcasting/WebSockets/SSE, polling,
ordered typed stream events, subscription authorization, backpressure, and
reconnection. It depends on actions, wire security, browser scheduling, and
morph identity. Ordinary HTTP actions remain the only transport for
authoritative island HTML and snapshots. The standalone reference host proves
the transport-neutral contract without claiming active Suprnova broadcasting
integration.

## Capabilities

### Typed component and browser events

Components shall dispatch and listen for registered events with typed bounded
payloads and explicit scope. Events coordinate already-owned behavior; they do
not bypass actions, model permissions, or authorization.

Acceptance criteria:
- Event names, payload schemas, source, target, and propagation scope are
  generated or registered.
- Scope can target self, parent, child, named island, document, or approved
  browser listener without accidental global broadcast.
- Unknown or malformed events are rejected.
- Event listeners that perform server-authoritative work invoke registered
  action/lifecycle paths.
- Browser events use safe structured data and never executable code strings.
- Pushed browser events enter through a core-validated registered-event
  capability that verifies current island, event schema, source, target, scope,
  fanout, and lifecycle before dispatch. The optional transport artifact cannot
  become a second event registry or global event bus.
- Event cycles and fanout are bounded.

UX flow:
1. Accepted component action dispatches an event -> registered targets receive
   it in defined order.
2. No valid target exists -> the event is ignored or diagnosed according to its
   delivery contract without invoking arbitrary code.

### Authorized broadcast subscriptions

Live islands may subscribe through a conforming host real-time adapter, with
current authentication, channel authorization, tenant isolation, and parameter
validation. The server emits a signed bounded subscription descriptor rather
than allowing directives to construct endpoints or channel names.

Acceptance criteria:
- Channel names and parameters derive from current registered server metadata
  plus bounded validated mount parameters rather than arbitrary directive
  interpolation. Registered topic templates may substitute only an exact
  `:parameter` path segment from that trusted parameter set.
- The descriptor binds registered stream identity, protocol/capabilities,
  topics, full allowed typed-event contracts (name, version, stable payload
  identity, schema, source, propagation targets, ordering, cycle policy, and
  fanout), authorization-context memo, authoritative baseline epoch/sequence,
  expiry, reconnect policy, and a bounded default hybrid poll fallback.
- The issuance request cannot propose its baseline. After resolving current
  registry scope, the host continuity authority supplies the first required
  position. Trusted registration rejects metadata whose calculated worst-case
  canonical claims exceed the descriptor budget.
- Private and presence subscriptions reauthorize the current principal.
- Subscription tokens are unique, cryptographically unpredictable, scoped,
  expiring, non-loggable, and atomically single-use when required. After every
  registry, scope, expiry, signing, and current-policy check passes, one
  host-owned operation consumes Connect while persisting its Renew successor, or
  consumes Renew while persisting its Connect successor. Replay and uniqueness
  authority belongs to that provider across processes and restarts. Provider
  failure is all-or-nothing and leaves the predecessor valid. A committed
  rotation whose response is lost leaves the predecessor consumed; the client
  must obtain a freshly issued subscription rather than replay or recover an
  idempotent rotation result.
- Cross-process fanout preserves tenant and channel isolation.
- Removing or navigating away from an island unsubscribes it.
- A push message cannot supply trusted snapshot or replacement HTML directly.

UX flow:
1. Eligible island connects -> runtime establishes its authorized subscription.
2. Authorization fails or changes -> subscription stops and the island exposes
   degraded freshness or its declared denial state.

### Push-triggered refresh and presentation

A server-pushed event shall trigger only a registered refresh, typed browser
event, or presentation-only local-signal update. Any authoritative state change
and HTML shall travel through normal Live verification, scheduling, rendering,
and morph contracts.

Acceptance criteria:
- Push metadata identifies registered response/presentation behavior, not an
  arbitrary method, action, effect, or JavaScript invocation.
- Push-triggered work enters the owning island scheduler and respects current
  revision.
- Burst events can coalesce refreshes without losing required state transitions.
- Authorization is rechecked when fresh protected data is rendered.
- A pushed browser event does not automatically invoke a domain-mutating Live
  action. Server-owned reactions use normal server event handlers; application
  users invoke registered mutating actions deliberately.
- Presentation-only stream data cannot write component, authorization,
  revision, accepted-outcome, or domain state.
- HTTP action transport remains available when push is absent.

UX flow:
1. Relevant server event arrives -> island queues one declared refresh or
   behavior.
2. Fresh render succeeds -> bounded morph updates the island without rerendering
   the document.

### Polling

Applications shall be able to declare bounded polling for state that benefits
from periodic refresh without a persistent connection. Polling shall pause or
reduce work when hidden, offline, disconnected, or superseded by another update
mechanism according to policy.

Acceptance criteria:
- Interval limits, jitter, visibility behavior, immediate/initial behavior, and
  fresh-render target are explicit. Polling never names or invokes a Live action.
- Poll requests enter the island scheduler and never overlap unsafely by default.
- Polling stops when its scope is removed or unauthorized.
- Server cache and conditional mechanisms may avoid unchanged render work.
- An application can expose stale/freshness status when polling is material.
- Applications may select polling-only, push-only, or hybrid policy. Under the
  default hybrid policy polling pauses only while push continuity is proved and
  resumes with bounded jitter whenever continuity is uncertain.
- The signed subscription descriptor supplies the default hybrid fallback
  interval. A legal `live:poll` on the same island may override it; `push-only`
  conflicts with `live:poll`.

UX flow:
1. Poll interval elapses in an eligible document -> runtime requests the
   registered refresh under scheduling policy.
2. Document hides or network fails -> polling pauses/backs off and resumes
   without a request storm.

### SSE/WebSocket typed event streams

Long-lived transports may deliver ordered bounded typed events, invalidations,
or presentation-only progress for a declared island stream. Streaming shall not
turn the component into an unbounded persistent server object or introduce a
second HTML, snapshot, revision, or DOM-patch protocol.

Acceptance criteria:
- Stream setup authenticates and authorizes its principal, tenant, component,
  and topic.
- Messages carry epoch/sequence, stream identity, size limits, and typed
  payloads plus a bounded subscription identity. A signed server baseline binds
  initial SSR state to the first required event; absent replay from that
  position requires refresh.
- The browser owns one physical document transport per compatible `(origin,
  transport, authorization scope)` and multiplexes island subscriptions through
  it. SSE uses an authenticated same-origin membership control path around a
  non-authority document-transport handle; WebSocket uses bounded
  subscribe/unsubscribe frames.
- A cookie-authorized WebSocket upgrade rejects missing, null, or unapproved
  `Origin` before accepting the connection. Explicit cross-origin deployment
  requires a configured non-wildcard allowlist and separate credential contract;
  wildcard origin acceptance is forbidden.
- SSE and WebSocket share one independently versioned event-envelope schema;
  transport choice cannot change message authority or continuity semantics.
- An invalidation or authoritative change enters the normal island scheduler and
  obtains HTML and snapshot state through an ordinary verified refresh/action
  response.
- Presentation-only data such as token-by-token text or progress may update a
  declared local signal but cannot become component, authorization, or domain
  state.
- Stream messages never carry trusted replacement HTML, snapshots, executable
  effects, or keyed DOM fragments.
- Gaps, duplicates, reconnects, and stream completion have explicit behavior.
- Backpressure bounds server buffers and slow-client resource use.
- Stream lifetime, cancellation, heartbeat, and deployment shutdown are
  observable and bounded.
- Connections, subscriptions, messages, replay windows, fanout, reconnects,
  fallback polls, and browser queues have explicit count/byte/time bounds.
- Persisted `pagehide` closes long-lived transports and transport timers before
  bfcache. `pageshow` reauthorizes and establishes a new physical connection
  before currentness may be reclaimed.

UX flow:
1. Application starts a declared stream -> the region exposes connected and
   progress state while ordered typed events arrive.
2. Sequence gap or disconnect occurs -> runtime pauses application, reconnects
   or refreshes authoritative island state, and never invents missing success.

### Reconnection and degraded freshness

Real-time loss shall not disable ordinary Live actions or document navigation.
Features whose correctness depends on pushed freshness shall expose connection
and staleness state and obtain current server state after uncertain gaps.

Acceptance criteria:
- Reconnect uses bounded exponential backoff and jitter.
- Resume tokens or sequence positions are used only when the backend proves
  continuity.
- Otherwise the island refreshes before claiming current status.
- The browser distinguishes disconnected, connecting, current, degraded,
  reconnecting, and closed; reconnection alone never changes degraded to
  current.
- Duplicate connections and subscriptions are detected after browser restore.
- Connection state is accessible but not noisy for features where it is
  immaterial.
- Global outages do not cause synchronized reconnect storms.

UX flow:
1. Push transport disconnects -> HTTP interactions continue and material regions
   show reconnecting/stale state.
2. Continuity is restored or disproven -> stream resumes from proof or the
   island refreshes to current authorized state.

## Acceptance criteria

- Events are typed, scoped, bounded, and cannot bypass registered behavior.
- Push subscriptions and refreshes reapply current authorization and scheduling.
- Polling and streaming have bounded lifetime, ordering, and backpressure.
- Gaps and reconnects never present uncertain data as current.
- Ordinary HTTP actions remain functional without real-time transport.

## Decisions and revisions

- 2026-08-25 -- Replaced claimed replay ranges with a bounded transcript of
  already membership- and registry-validated envelopes. Gap recovery requires
  every same-scope, same-epoch position from the last applied successor through
  at least the recorded high-water with no empty proof, duplicate, regression,
  or omission. A new epoch or otherwise unavailable replay can be adopted only
  through the injected host continuity authority, and its baseline must not
  regress and must cover all observed high-water.
- 2026-08-25 -- Implemented the independent canonical async-envelope protocol
  v1 with required bounded logical-subscription identity, registered stream,
  monotonic epoch/sequence position, and the closed refresh, typed browser
  event, declared presentation signal, heartbeat, completion, or error payload
  union. The sequence machine applies only the exact same-epoch successor,
  ignores duplicates and older epochs, preserves its last authoritative
  position across gaps and newer epochs, and cannot restore currentness or
  adopt a non-regressing baseline without a complete validated replay
  transcript or an authoritative host refresh.
- 2026-08-25 -- Implemented the canonical subscription-v1 descriptor with the
  exact `suprnova-live/async-subscription/v1` HKDF purpose, bounded exact-key
  claims, overlapping key-ID verification, exclusive expiry, and a
  principal/session/tenant/component-contract context memo. Full event
  contracts are signed rather than event names alone. Issue, connect, and renew
  independently re-resolve the current component contract, stream, event
  contracts, and topics; topic templates accept only bounded trusted mount
  parameters whose canonical segments reject empty, encoded-separator, and
  traversal forms. Issuance obtains its baseline only from the host continuity
  authority, and trusted registration proves the worst-case full claims fit the
  canonical descriptor budget. A separate zeroizing, unique credential binds
  the exact descriptor, current subscription scope, expiry, and Connect or Renew
  operation. Only after all non-mutating checks succeed, one host-provider
  transaction consumes the predecessor and persists a unique successor across
  processes and restarts. Atomic provider failure retains the predecessor;
  committed-but-lost responses require fresh issuance rather than a Task 3
  idempotent rotation machine.
- 2026-08-24 -- Multiplexed compatible subscriptions over one document transport,
  made subscription identity explicit in every envelope, and required strict
  WebSocket `Origin` validation. Polling is fresh-render-only; signed descriptors
  provide hybrid fallback defaults, with a legal `live:poll` override and a
  `push-only` conflict. Pushed browser events cross only the core-validated
  registered-event port, and persisted pagehide closes transports before bfcache.
- 2026-08-23 -- Locked Iteration 004 asynchronous updates to independently
  versioned typed SSE/WebSocket envelopes, signed subscription descriptors, and
  polling-only, push-only, or continuity-aware hybrid freshness. Push may queue
  only registered refresh, typed browser event, or presentation-only signal
  work; it never automatically invokes a mutating Live action. Gaps require
  complete validated replay transcript or authoritative host refresh before
  currentness is claimed;
  the signed descriptor's baseline epoch/sequence closes the initial SSR-to-
  stream gap.
- 2026-08-21 -- WebSockets and SSE augment Live; rejected persistent sockets as
  the foundation for component state.
- 2026-08-21 -- Push messages trigger declared verified behavior rather than
  carrying trusted arbitrary replacement HTML.
- 2026-08-21 -- Streaming transports carry typed events, invalidations, and
  presentation-only local data. Rejected streamed HTML fragments and automatic
  browser-triggered domain mutations as second authority paths.
