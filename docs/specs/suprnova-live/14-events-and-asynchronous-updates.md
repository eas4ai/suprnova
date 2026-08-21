# Suprnova Live -- 14 Events and Asynchronous Updates

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns declared component/browser events, cross-island communication,
server-pushed refresh through Suprnova broadcasting/WebSockets/SSE, polling,
ordered typed stream events, subscription authorization, backpressure, and
reconnection. It depends on actions, wire security, browser scheduling, and
morph identity. Ordinary HTTP actions remain the only transport for
authoritative island HTML and snapshots.

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
- Event cycles and fanout are bounded.

UX flow:
1. Accepted component action dispatches an event -> registered targets receive
   it in defined order.
2. No valid target exists -> the event is ignored or diagnosed according to its
   delivery contract without invoking arbitrary code.

### Authorized broadcast subscriptions

Live islands may subscribe to Suprnova broadcast channels through the existing
real-time infrastructure, with current authentication, channel authorization,
tenant isolation, and parameter validation.

Acceptance criteria:
- Channel names and parameters derive from trusted server metadata rather than
  arbitrary directive interpolation.
- Private and presence subscriptions reauthorize the current principal.
- Subscription tokens are scoped, expiring, non-loggable secrets when required.
- Cross-process fanout preserves tenant and channel isolation.
- Removing or navigating away from an island unsubscribes it.
- A push message cannot supply trusted snapshot or replacement HTML directly.

UX flow:
1. Eligible island connects -> runtime establishes its authorized subscription.
2. Authorization fails or changes -> subscription stops and the island exposes
   degraded freshness or its declared denial state.

### Push-triggered refresh and actions

A server-pushed event shall trigger only a registered refresh, event, or action
behavior. Any resulting state change and HTML shall travel through normal Live
verification, scheduling, rendering, and morph contracts.

Acceptance criteria:
- Push metadata identifies a registered response behavior, not arbitrary method
  or JavaScript invocation.
- Push-triggered work enters the owning island scheduler and respects current
  revision.
- Burst events can coalesce refreshes without losing required state transitions.
- Authorization is rechecked when fresh protected data is rendered.
- A pushed browser event does not automatically invoke a domain-mutating Live
  action. Server-owned reactions use normal server event handlers; application
  users invoke registered mutating actions deliberately.
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
  target action are explicit.
- Poll requests enter the island scheduler and never overlap unsafely by default.
- Polling stops when its scope is removed or unauthorized.
- Server cache and conditional mechanisms may avoid unchanged render work.
- An application can expose stale/freshness status when polling is material.

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
- Messages carry sequence, stream identity, size limits, and typed payloads.
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

- 2026-08-21 -- WebSockets and SSE augment Live; rejected persistent sockets as
  the foundation for component state.
- 2026-08-21 -- Push messages trigger declared verified behavior rather than
  carrying trusted arbitrary replacement HTML.
- 2026-08-21 -- Streaming transports carry typed events, invalidations, and
  presentation-only local data. Rejected streamed HTML fragments and automatic
  browser-triggered domain mutations as second authority paths.
