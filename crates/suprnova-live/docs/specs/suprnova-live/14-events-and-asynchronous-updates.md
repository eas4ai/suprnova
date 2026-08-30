# Suprnova Live -- 14 Events and Asynchronous Updates

Status: Normative design specification
Last revised: 2026-08-26

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
- One exhaustive browser dispatcher consumes only canonical membership-validated
  envelopes. Its three productive branches use the slot-specific async island
  port for fresh render, a core-minted registered-event capability, or the exact
  signed `(signal scope, name, type)` presentation write. Lifecycle payloads
  return bounded reasons and only observe, close, or degrade the exact logical
  membership.
- Push-triggered work enters the owning island scheduler and respects current
  revision.
- Refresh admission is pending until normal response validation, morph, and
  commit-after-morph reach a terminal scheduler outcome. Failure, cancellation,
  or retirement degrades at the already-consumed stream high-water and requests
  authoritative recovery without replaying a mutating operation. Burst events
  can coalesce by the exact island semantic key without losing this completion.
  The observer captures the exact subscription continuity/lifecycle generation
  at admission; a later transport loss, heartbeat loss, authorization rotation,
  cancellation, retirement, or reauthorization fences an older success terminal
  from committing sequence state or restoring currentness.
- Coalesced refresh completion uses one scheduler-intent callback plus a bounded
  owner table keyed by exact active logical membership. A newer admission for the
  same membership replaces that slot; it never appends another intent callback.
  Saturation reports exhaustion and degrades the affected membership rather than
  throwing or claiming completion.
- Live delivery and reconnect replay use the same terminal observer. A replay
  that reaches a refresh pauses later presentation effects and cannot finish or
  advance through that refresh until validation, morph, and commit-after-morph
  succeed. Terminal failure preserves the truthful committed prefix and consumed
  high-water, degrades only that membership, and requires authoritative recovery.
- Authorization is rechecked when fresh protected data is rendered.
- A pushed browser event does not automatically invoke a domain-mutating Live
  action. Server-owned reactions use normal server event handlers; application
  users invoke registered mutating actions deliberately.
- Presentation-only stream data cannot write component, authorization,
  revision, accepted-outcome, or domain state.
- Core binds registered-event authority to the exact island owner and rechecks
  owner currentness and capability rotation after bounded target resolution.
  It first snapshots the complete caller dispatch candidate and nested payload
  exactly once into bounded immutable own-data records, rejecting accessors,
  inherited or symbol fields, sparse structures, extra fields, and inconsistent
  traps before validation or use. Core then
  constructs the DOM `Event`, then immediately rechecks the connected source and
  every distinct target against the exact runtime document, registry record,
  immutable component/document-key/slot/root metadata, captured ownership scope,
  current parent/child/named relationship, and current capability before each DOM
  dispatch.
  Forged, stale, detached, cross-island, wrong-scope, over-fanout, cyclic, or
  retired delivery fails without sequence commit.
- A fanout whose first target observes an event before a later target becomes
  invalid returns bounded delivered/skipped counts and a closed reason. That
  observable prefix is not rolled back, but the sequence remains uncommitted and
  degraded at a non-replayable consumed high-water. Automatic replay that
  includes or crosses that position is rejected so it cannot duplicate the
  prefix. Recovery requires a trusted authoritative no-tail baseline at or after
  the consumed position before later successors may dispatch.
- A presentation-signal contract accepts only null, boolean, string, or
  browser-safe signed/unsigned integer values. Its scope identity is 1--128 ASCII
  bytes, starts alphanumeric, and then permits only alphanumeric, `.`, `_`, `:`,
  or `-`; slash and leading punctuation are invalid. Its distinct signal name is
  1--64 ASCII bytes, starts with lowercase `a`--`z`, and then permits only
  lowercase letters, digits, `.`, `_`, or `-`; uppercase, slash, leading digit or
  punctuation, Unicode, and a 65th byte are invalid. Core rechecks the exact
  connected declared scope element and owner immediately before the write.
- HTTP action transport remains available when push is absent.
- SSE unsubscribe rejection, timeout, cancellation, or late completion settles
  that logical cleanup locally with bounded diagnostics. It never fails or
  closes the pooled physical source, degrades a sibling membership, or requests
  sibling reauthorization; subscribe admission failure may still fail the
  transport when its authentication contract requires it.
- After presentation failure, the exact active membership retains its physical
  document-transport identity, subscription identity, prior authenticated
  transport generation, and descriptor binding as a local degraded-lane fence.
  Frames for only that same physical group and known lane are discarded while
  successor acknowledgment is pending. Suspend, replacement, removal, or close
  clears the fence, so a replacement group cannot inherit it merely because its
  numeric generation collides; foreign, never-authenticated, wrong-group,
  wrong-generation, or wrong-binding traffic retains the physical
  authorization-failure contract.
- Fresh-render scheduler exhaustion is a distinct `resource_exhausted`
  presentation outcome. It degrades and reauthorizes only the exact membership,
  reports one bounded resource diagnostic, activates the signed hybrid fallback,
  and leaves push-only timer-free; it is not collapsed into or double-reported as
  a generic dispatcher rejection.

UX flow:

1. Relevant server event arrives -> island queues one declared refresh or
   behavior.
2. Fresh render succeeds -> bounded morph updates the island without rerendering
   the document.

### Polling

Applications shall be able to declare bounded polling for state that benefits
from periodic refresh without a persistent connection. Polling shall pause when
hidden, offline, disconnected, or superseded by another update mechanism
according to policy. Hidden or offline polling owns no retry timer; the next
eligible environment transition schedules normal jitter without catch-up work.

Acceptance criteria:

- Interval limits, jitter, visibility behavior, immediate/initial behavior, and
  fresh-render target are explicit. Polling never names or invokes a Live action.
- Poll requests enter the island scheduler and never overlap unsafely by default.
- Poll success and failure backoff follow actual fresh-render HTTP, protocol,
  recovery, and application completion from that scheduler intent. Admission or
  coalescing alone never resets failure state.
- Polling stops when its scope is removed or unauthorized.
- Every async-enabled island, including one with no initial freshness directive,
  owns the async lifecycle bridge. Only a committed morph rescans directives
  owned by that island; an aborted or failed morph preserves the prior policy.
  Removal retires its timer and pending completion, addition starts the resolved
  policy, and interval, visibility, initial, or stream-mode changes replace the
  prior generation atomically. Nested child-island directives remain child-owned.
- Server cache and conditional mechanisms may avoid unchanged render work.
- An application can expose stale/freshness status when polling is material.
- Applications may select polling-only, push-only, or hybrid policy. Under the
  default hybrid policy polling pauses only while push continuity is proved and
  resumes with bounded jitter whenever continuity is uncertain.
- The signed subscription descriptor supplies the default hybrid fallback
  interval. A legal `live:poll` on the same island may override it; `push-only`
  conflicts with `live:poll`.
- Exact membership acknowledgment and validated replay or authoritative-no-tail
  proof establish continuity before an immediate hybrid timer can start. A
  current initial membership therefore emits no speculative refresh; late or
  replaced proofs remain generation-fenced.
- A committed morph that changes owned freshness directives while initial or
  replacement membership proof is pending records only the newest policy
  intent. It cannot apply pending authorization, start polling, or dispatch
  staged replay/effects. A changed intent immediately retires any active
  degraded fallback from the older policy. The exact accepted acknowledgment
  applies the newest intent atomically against committed authorization. If
  replacement membership fails, the newest intent may resume in degraded mode
  only against the last committed authorization: removal or push-only stays
  timer-free, while a changed hybrid policy uses its new interval. Removal,
  replacement, or a fail-closed directive conflict fences every older intent,
  timer, and late completion.
- `suspended` and `closed` are stronger lifecycle states than connectivity or
  continuity. Creating, starting, updating, or receiving a late continuity
  callback for a timer while suspended cannot report `degraded`, `offline`, or
  `current` and cannot arm work. Resume recomputes the latest committed policy
  once and applies ordinary jitter without reviving an older generation.
- One optional configured observer receives immutable island identity plus the
  closed `current`, `degraded`, `polling`, `offline`, `suspended`, or `closed`
  semantic freshness state only when it changes. It is presentation/accessibility
  input, not mutable authority or an unbounded event subscription surface.

UX flow:

1. Poll interval elapses in an eligible document -> runtime requests the
   registered refresh under scheduling policy.
2. Document hides or network fails -> polling pauses without an eligibility
   retry timer and resumes with normal jitter, without a catch-up request storm.

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
transport, document authorization scope)` and multiplexes island subscriptions
  through it. The document scope is a compact collision-resistant derivation of
  trusted aggregate scope, principal, session, tenant, and explicit host
  transport-policy identity; it excludes component name and component contract,
  which remain isolated in each logical membership's authorization memo. SSE
  uses an authenticated same-origin membership control path around a
  non-authority document-transport handle; WebSocket uses bounded
  subscribe/unsubscribe frames. Compatibility also requires the exact current
  cookie/bearer credential contract and the commutative strict intersection of
  every member's reconnect bounds. A physical group never adopts whichever
  island happened to register first as credential or retry-policy authority.
- A cookie-authorized WebSocket upgrade rejects missing, null, or unapproved
  `Origin` before accepting the connection. Explicit cross-origin deployment
  requires a configured non-wildcard allowlist and separate credential contract;
  wildcard origin acceptance is forbidden.
- SSE and WebSocket share one independently versioned event-envelope schema;
  transport choice cannot change message authority or continuity semantics.
- Every external SSE or WebSocket membership add/remove enters a trusted host
  transport-authority port. At that exact consumption boundary the host
  re-resolves the current component contract and authorization memo (principal,
  session, tenant, and aggregate scope), active logical membership, registered
  stream, resolved topics, full event contracts, canonical subscription modes,
  and exact document/control authority; the framework independently rechecks
  exclusive descriptor expiry, the current document authorization scope, and a
  compact binding of the exact signed Task 2 descriptor wire. That binding
  includes signing key ID and signature, so equal claims signed during key
  rotation cannot replace, remove, or control one another's membership.
  A retained descriptor-bound request, document handle, or browser control is
  never reusable current authority.
- Subscription establishment is a split one-use control operation: synchronous
  preparation snapshots the exact document tuple and control generation; an
  owned pending operation obtains fresh authority without borrowing the
  document; a synchronous pre-source gate rechecks generation, physical scope,
  exact server-owned document instance, expiry, duplicate/retirement fences,
  and capacity; source establishment and
  post-subscribe authority then run without a document borrow; and one
  synchronous commit repeats those checks immediately before mutation. A
  pending authority/source operation therefore cannot block delivery or another
  control. Failed or canceled post-subscribe validation and failed/stale commit
  close/dispose the newly opened logical session exactly once and install
  nothing. External removal uses the same prepare/authorize/commit split. It
  authenticates before classifying unknown or descriptor-mismatched local
  membership, while completion, revocation retirement, cancellation recovery,
  and controlled shutdown retain internal cleanup authority after browser
  credentials expire. Pending external controls are document-owned through
  one-use permits and have a hard count bound independent of Task 5 fanout.
- Logical completion, source failure, routing failure, removal, and shutdown
  detach a membership from active routing before cleanup. The document retains
  cleanup ownership in the same hard-bounded retirement lane and polls it fairly
  through a persistent executor-neutral interface; a pending or failing close
  cannot stall active siblings, monopolize a wake, spawn an unowned task, or
  permit post-terminal delivery. A retiring entry retains the exact
  subscription ID and signed-descriptor binding as a fence until cleanup
  succeeds, so it consumes capacity, rejects same-ID re-admission under either
  the same or an overlapping-key binding, and cannot clean up a later
  replacement. Active plus retiring sessions share the same membership ceiling.
- Current registered `SubscriptionModes` are authority. The physical document
  kind is compatibility only: SSE-only cannot use WebSocket, WebSocket-only
  cannot use SSE, and any same-name mode-set revision invalidates a retained
  membership request even when the newly registered set still contains the
  invoked adapter.
- Queue admission rechecks current descriptor expiry, logical membership,
  subscription/stream scope, full event contracts, and declared presentation
  signals for every envelope. A cloneable decode context is not fresh
  membership authority. Guard consumption rechecks exclusive descriptor expiry
  so a retained once-current guard cannot dispatch after expiry.
- Sequence classification precedes registered dispatch. Exact-next delivery
  advances only after dispatch succeeds; dispatch rejection or failure retains
  the prior position so a fresh retry is not misclassified as a duplicate.
- Browser fanout partial delivery is a failed dispatch, not success: the bounded
  delivered prefix is diagnostic/observable, the current sequence does not
  advance, the consumed position becomes recovery high-water, and later delivery
  remains inert until authoritative recovery. The runtime never automatically
  retries that partially delivered event or accepts a replay crossing its
  non-replayable consumed position; only an authoritative no-tail baseline that
  absorbs that position can restore currentness.
- Replay validates the entire bounded same-scope transcript before any dispatch
  or mutation only when the exact lane already has a sequence or pressure
  recovery obligation. A healthy lane rejects every transcript before host
  clock or registry work. Count, payload, aggregate bytes, and queue capacity
  preflight before bounded local work. The document then resolves the exact
  stored active authorization by binding and document scope and validates its
  immutable context, registered payload/event/signal/target contract, exact
  scope, and contiguous coverage before any host callback. A reconstructed
  authorization or substituted clock is not stored authority. Only then may the
  stored clock and one atomic current-membership registry snapshot run; invalid
  evidence is a typed input rejection and never a new `Degraded` pressure
  outcome. Delivery then final-validates and immediately
  dispatches each entry in order, with no host callback between that entry's
  accepted authority and registered dispatch. Partial failure, authorization
  loss, cancellation, or retirement reports its applied prefix, current
  position, state, and effective required high-water, including pressure-only
  recovery while the sequence lane itself is current, independently of the outer
  failure kind. Nested interruption kinds distinguish expiry, current-authority
  loss, delivery retirement, and registered-dispatch failure.
  The exact recovery obligation remains degraded and resumes only from freshly
  admitted remaining suffix evidence. Replay carries presentation data only;
  `Complete` is accepted only from the live provider path because lifecycle
  detachment is not replayable.
- An invalidation or authoritative change enters the normal island scheduler and
  obtains HTML and snapshot state through an ordinary verified refresh/action
  response.
- Presentation-only data such as token-by-token text or progress may update a
  declared local signal but cannot become component, authorization, or domain
  state.
- Stream messages never carry trusted replacement HTML, snapshots, executable
  effects, or keyed DOM fragments.
- Gaps, duplicates, reconnects, and stream completion have explicit behavior.
  `Complete` is terminal: the membership is detached before its single terminal
  envelope is returned and all later provider items are ignored. A typed
  `Error` payload is nonterminal protocol information; a source/session failure
  is terminal and retires only that logical membership.
- Backpressure bounds server buffers and slow-client resource use.
- Server delivery pressure is one policy wrapper over the shared resource owner,
  bounded queue, permit pool, and cancellation flag. It does not create a
  second queue, permit counter, lifetime owner, detached worker, or sequence
  authority. One owning document delivery queue retains at most 64 unapplied
  envelopes and 256 KiB of canonical envelope bytes across all of that
  document transport's logical memberships. The document transport polls one
  logical source fairly and immediately offers that one item; Live owns no
  hidden per-membership ingress buffer in front of the aggregate queue. A host
  provider's native internal buffers are outside this trait contract.
- The document-owned delivery operation alone may create a private sealed async
  buffer entry from the exact active document membership. Neither application
  code nor an external buffer caller can mint, retain, or later offer that
  proof. The operation first seals bounded facts, then immediately revalidates
  them against the current document generation and host authority before one
  synchronous queue mutation; no await, callback, or public capability occurs
  between final acceptance and commit. Admission rechecks exclusive
  descriptor expiry, exact subscription binding, document authorization scope,
  component authorization memo, active logical routing membership, registry
  and revocation state, the full current event/signal contract, and the exact
  envelope scope. The queued entry owns the one-use Task 3 membership guard.
  Dequeuing creates a private non-cloneable delivery lease that retains that
  proof, its shared permit/cancellation state, exact membership scope, and the
  document pressure-continuity tracker. The document owns exactly one Task 3
  sequence machine lane for each exact logical subscription binding and selects
  that lane itself; callers cannot provide a different machine or report
  success. Raw envelope admission, sequence mutation, and
  `DocumentTransportSession` envelope delivery are private; every public
  provider-delivery and recovery path enters the bounded document owner. Commit
  or dispatch time is captured before the final current-host validation and is
  passed through that validation; no host callback may occur between the final
  accepted facts and synchronous queue mutation or registered dispatch.
  Successful apply, duplicate, and stale-epoch outcomes resolve truthfully.
  Authority loss, cancellation, gaps, epoch change, dispatcher failure, or an
  unresolved lease drop mark pressure continuity degraded and never advance
  sequence falsely. The synchronous closed document owner does not expose a
  second cancellation capability: aggregate retirement cannot interleave through
  another public mutable path while registered dispatch owns `&mut self`; the
  private lease still observes its shared retirement flag and reports the exact
  delivery-retired recovery progress if internal lifecycle cancellation occurs.
- For a browser event, the trusted host registry supplies the current resolved
  nonzero recipient count and an exact target-set scope digest. The browser and
  buffer caller cannot propose fanout. Admission rejects target-count, target,
  contract, binding, document-scope, memo, expiry, or revocation drift before
  queue mutation and before per-target cloning. The resolved count must satisfy
  both the full current event contract and deployment policy; self, parent, and
  exact named-island targets resolve to exactly one recipient. Registered
  dispatch consumes a private-construction resolved-delivery capability carrying
  the exact accepted target-scope digest, resolved count, and deployment limit;
  the dispatcher cannot substitute caller-proposed recipients.
- Admission checks the 32 KiB canonical payload ceiling, replay count and
  aggregate bytes, current descriptor-bound event fanout, deployment fanout
  policy, queue count/bytes, and owner cancellation before per-target cloning or
  delivery work. Delivery acquires one shared permit before removing a queued
  envelope, so saturation leaves the queue unchanged. Replay is one exact
  subscription-binding, document-scope, component-memo, stream, and epoch
  transcript with contiguous positions. Empty, global/configured over-count,
  and aggregate item overflow are rejected before internal transcript
  allocation or any host/registry callback; that hard count bound makes later
  payload and aggregate-byte validation itself bounded. Its complete checked
  byte reservations commit under one shared queue critical section; rejection,
  cancellation, or a concurrent cloned-handle admission changes no replay queue
  position. Private contiguous group markers preserve the admitted transcript
  through one lock-scoped dequeue and one RAII permit lease. Dispatch invokes
  Task 3 replay recovery on the exact lane, never ordinary single-envelope
  dispatch. A pressure loss may leave that Task 3 lane current; in that case the
  same machine validates and commits the exact successor transcript through the
  pressure tracker's authorized lost high-water rather than creating a second
  counter. A later replaceable ordinary message cannot coalesce with or replace
  any replay group member. Unresolved pressure is retained by exact subscription
  binding, document/component scope, and finite cause. Complete recovery clears
  only that exact membership's covered causes, and only after the aggregate queue
  is empty; document currentness additionally requires every exact logical
  sequence lane and every unresolved pressure cause to be clean. One membership
  cannot clear a sibling's pressure loss even when both Task 3 lanes otherwise
  report current. Admission, delivery, sequence, and detachment are the four
  finite cause classes, so retained cause storage is hard-bounded at four times
  the document membership ceiling across lifetime churn. Saturation stays
  conservatively degraded until aggregate retirement rather than discarding an
  unknown obligation. Authorization or dispatcher failure retains the exact
  unresolved cause and exposes the truthful committed replay prefix.
- A freshly authorized document-owned authoritative refresh may recover the
  exact private sequence lane and only that membership's covered pressure
  causes. Before the continuity callback, the document resolves the exact stored
  active authorization and callback-free compares the caller's signed context,
  origin, document scope, binding, and authority identity. The host then
  proposes the baseline; commit time from the stored clock and exact current
  scope/expiry/registry validation follow as the final host callback, after
  which callback-free baseline installation and pressure recovery occur. The
  baseline cannot regress and must cover both sequence and pressure high-water.
  Under the `u64` sequence vocabulary there is no same-epoch value after
  `u64::MAX`: equal and lower values are duplicates, while a greater position
  necessarily enters a new epoch and requires authoritative recovery.
- Coalescing may replace only the newest exact contiguous refresh for the same
  signed-descriptor binding, document authorization scope, component memo,
  subscription, stream, and epoch, or presentation signal with that same scope
  and registered signal identity and schema contract. Semantic comparison and
  replacement occur under the same shared queue lock, so a cloned handle cannot
  redirect replacement to a changed tail. A redundant equal-or-older
  replaceable tail is retained without inventing a delivery loss. Actual
  successor replacement retains the latest envelope but marks continuity
  degraded because an earlier sequence was superseded.
  Required ordered browser events, heartbeats, completion, and errors never
  coalesce; pressure never evicts one while claiming continuity.
- Admission returns the closed `Queued`, `Coalesced`, `Degraded`, or
  `Closed(code)` disposition. Terminal policy violations cancel and drain the
  owning delivery scope once, including closure produced during replay
  admission; later pumps perform no provider or authority read.
  A provider item extracted before an authority callback remains under an RAII
  loss guard, so panic or cancellation before queue ownership records truthful
  pressure loss and releases resources. Telemetry uses only the finite queued,
  coalesced, degraded, closed, rejected, and cleanup labels; subscription,
  stream, event, principal, payload, descriptor, and raw-error values are
  forbidden labels.
- Stream lifetime, cancellation, heartbeat, and deployment shutdown are
  observable and bounded.
- Explicit membership removal and provider failure atomically purge only queued
  entries with that exact subscription binding, release their byte/item
  reservations under the queue lock, and drop removed values after unlocking.
  Graceful source completion and `Complete` retain already-admitted ordered
  predecessors through the single terminal drain. While that drain retains its
  exact Task 3 lane, same-ID re-admission is fenced: the exact binding is a
  duplicate and a rotated binding is a descriptor mismatch. Delivery of the
  terminal predecessor (or an empty EOF) prunes the drain and lane before return,
  after which exact or signing-key-rotated identity reuse creates one fresh lane.
  Authenticated explicit removal discharges only that exact retired binding's
  remaining pressure obligation; unrelated detach/cleanup does not silently
  forgive lost continuity. A healthy sibling remains routable throughout cleanup.
- Connections, subscriptions, messages, replay windows, fanout, reconnects,
  fallback polls, and browser queues have explicit count/byte/time bounds.
- Every ordinary reconnect and bfcache restore of a committed logical membership
  reauthorizes that exact membership at its current position and requires
  complete replay or an authoritative no-tail proof before delivery may be
  current. Document-owned
  reauthorization uses at most eight fair concurrent calls, gives each call an
  owned abortable deadline, and generation-fences late noncooperative results.
  A call's execution deadline starts only after document-scheduler admission;
  queued time cannot spend that deadline. Initial and committed-recovery work
  alternate when both are pending, so neither source can continually front-load
  the other. Suspension or retirement cancels queued work and aborts admitted
  work before any later source result can regain authority.
  Raw socket open never resets the retry counter; only authenticated transport
  membership plus later continuity evidence does. Reconnect policy therefore
  has a positive exponential base and terminally degrades after its attempt cap.
- SSE membership controls have bounded count and time, own their cancellation,
  and settle or abort on reconnect and retirement. Native EventSource remains
  cookie-only; bearer SSE uses the bounded fetch stream without a URL secret.
  WebSocket subscribe frames bind the exact subscription, stream, and signed
  descriptor digest rather than carrying a subscription ID alone. A logical
  membership becomes authenticated only after its bounded transport control
  resolves, or after the WebSocket host returns an exact post-commit membership
  acknowledgment bound to that connection's control nonce, stream, and transport
  generation. The host can mint that acknowledgment only by consuming the
  non-cloneable receipt emitted by the exact successful commit; current
  membership presence cannot mint another. Queueing or sending a control frame
  is not acknowledgment.
  Rejection, timeout, transport loss, cancellation, or a late/foreign
  acknowledgment cannot consume replay/no-tail proof or reset reconnect state.
  An uncommitted initial authorization remains inert but recoverable across a
  bounded pre-acknowledgment transport replacement; recovery never routes it
  through successor reauthorization against an intentionally absent committed
  predecessor. Every replacement takes its transport key, credential,
  transport kind, reconnect policy, and heartbeat policy from the exact staged
  effective authorization. A physical generation accepts only its first
  terminal open/failure transition and retires its callback token before
  invoking adapter cleanup, so reentrant or late callbacks cannot consume an
  attempt or replace an owned timer.
- Persisted `pagehide` closes long-lived transports and transport timers before
  bfcache. If the first exact membership acknowledgment has not committed yet,
  suspension discards that generation's staged authorization and acknowledgment
  authority while retaining only the fact that no predecessor exists.
  If the first authorization invocation is itself still pending and the island
  has not entered the document connection pool, suspension aborts and fences
  that invocation as well. On restoration, the document owner schedules only
  those non-pool-owned islands through the same fair, at-most-eight-wide
  authorization budget; pool-owned islands remain the connection pool's
  responsibility and are never requested twice.
  `pageshow` then invokes a fresh initial authorization request with `prior: null`
  and no inherited position, stages its raw/replay/no-tail result inertly, and
  establishes a new physical connection. Only that replacement connection's
  exact acknowledgment may commit the fresh initial authority or effects; old
  acknowledgments and late data remain inert. A committed membership instead
  follows current-position successor reauthorization before currentness may be
  reclaimed.

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
- If persisted-page restoration cannot reauthorize a previously committed
  hybrid membership, the old native socket remains closed and the last committed
  signed poll policy resumes once in degraded mode with normal jitter and no
  catch-up burst. Push-only exposes degraded state without polling; poll-only
  retains its independent lifecycle.
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

- 2026-08-26 -- Shared the complete final island-currentness guard across event
  source and self/parent/child/named targets, including exact runtime document,
  registry identity, immutable root metadata, captured ownership, and live
  relationship checks after event construction and before every fanout dispatch.
  Exact degraded memberships now quarantine their old-generation frames locally
  until successor acknowledgment without failing pooled siblings. Fresh-render
  capacity exhaustion remains a distinct exact-membership `resource_exhausted`
  lifecycle outcome with one diagnostic and ordinary hybrid/push-only fallback
  semantics. The degraded-lane quarantine is bound to the exact physical
  document-transport identity as well as subscription, descriptor, and generation,
  preventing a replacement group's reused numeric generation from inheriting it.
- 2026-08-26 -- Closed the remaining Task 8 review findings. Registered-event
  dispatch now snapshots the complete caller candidate and nested payload once
  before validation, and final delivery rechecks the post-construction connected
  source and each exact guarded target. SSE unsubscribe failures remain local to
  logical cleanup. Partial fanout records a non-replayable consumed position that
  only an absorbing authoritative baseline can recover; refresh terminals carry
  the dispatch lifecycle generation; and exact-membership reauthorization no
  longer double-advances that generation. Presentation signals now use a distinct
  lowercase-first 64-byte `SignalName`, and server coalescing includes exact
  scope, name, and schema.
- 2026-08-26 -- Hardened Task 8 after adversarial implementation review. Optional
  async code now receives a distinct frozen runtime port with no upload, model,
  state, or generic event-registration surface; core captures one immutable
  bounded registered-event snapshot, guards and revalidates every resolved
  target, and truthfully reports partial fanout. Stream refresh success waits for
  the existing scheduler's validation/morph/commit terminal outcome, while
  failure retains the consumed high-water and activates exact-membership
  recovery. One callback per scheduler intent fans out through a bounded
  exact-membership completion table, and replay serializes later effects behind
  the same terminal observer. Completion, partial delivery, and error retain
  bounded reasons and own exact membership, heartbeat, and hybrid-fallback
  lifecycle without reconnecting transport siblings. Event construction precedes
  the final connected source/target authority check. Presentation signals are
  registered, encoded, decoded, and dispatched by the shared bounded scope
  grammar plus name and the local-signal-only null/boolean/string/safe-integer
  schema rather than an island-root or nearest-scope default.
- 2026-08-26 -- Implemented one closed browser async dispatcher over the
  canonical membership-validated envelope union. Refresh enters the existing
  fresh-render scheduler, registered browser events consume a core-minted
  exact-island capability with a final check before every resolved target, declared
  presentation signals use the existing typed signal port, and lifecycle records
  only observe, close, or degrade their exact subscription. No action, effect,
  call, HTML, snapshot, revision, component-state, or second scheduler seam was
  added.
- 2026-08-26 -- Made suspended polling state authoritative over start, policy,
  environment, and continuity recomputation. A replacement stage discarded by
  page suspension may restore the newest committed policy object, but it remains
  suspended and timer-free until one lifecycle resume; late membership proof and
  older timer completion stay inert.
- 2026-08-26 -- Closed the replacement-membership morph race: a changed
  committed freshness intent immediately retires the older degraded fallback
  while successor membership proof remains pending. Exact acknowledgment applies
  only the newest policy; failure resolves that same intent against last
  committed authority and can never revive the old interval or completion.
- 2026-08-26 -- Deferred committed-morph freshness changes while exact initial
  or replacement membership proof is pending. The browser retains the latest
  generation-fenced owned directive intent, applies it only with committed
  authorization after the exact SSE/WebSocket acknowledgment, and prevents
  removed, conflicting, or stale immediate policies from starting later.
- 2026-08-26 -- Made freshness directives committed-morph lifecycle state for
  every async-enabled island, including initially directive-free islands. Only
  island-owned directives are rescanned after commit; removal, addition, and
  policy changes generation-fence obsolete timers and completions. Hidden and
  offline polling now pauses event-first with no eligibility retry timer and
  resumes under normal jitter without catch-up work.
- 2026-08-26 -- Hardened Task 7 around proof and completion truth: the generated
  v4 freshness table is now enforced by the real island-level Rust checker;
  replay/no-tail continuity is committed before an immediate hybrid timer;
  polling backoff observes the existing scheduler intent's actual terminal
  result; failed persisted-page reauthorization resumes the last committed
  hybrid fallback without socket reuse; and one immutable bounded freshness
  observer exposes semantic state without authority.
- 2026-08-26 -- Moved each authorization execution deadline after admission to
  the shared document scheduler. Initial and committed-recovery sources now
  receive alternating admission when both are pending, total admitted work
  remains at most eight, and suspend/dispose cancels queued work without letting
  queue wait consume a membership's execution budget.
- 2026-08-26 -- Closed the pre-install bfcache lifecycle gap. A pending first
  authorization is now abortable before the island enters the connection pool;
  persisted restoration makes a fresh, generation-fenced initial request with
  no predecessor or position. The document-wide authorization scheduler is
  shared by initial connect, pool recovery, and non-pool-owned restoration so
  mixed cohorts remain fair and at most eight-wide without duplicate requests.
- 2026-08-26 -- Kept uncommitted initial subscription authority inert and
  generation-owned across bounded pre-acknowledgment transport loss rather than
  invoking successor reauthorization without a committed predecessor. Bfcache
  suspension discards the old generation's stage but retains the no-predecessor
  lifecycle fact, so restoration makes a deadline-owned fresh initial request
  with `prior: null` rather than successor reauthorization. Reconnect grouping
  uses the complete staged effective transport authority, with compatible
  aggregation independent of completion order and deterministic rejection of
  incompatible rotations. Each physical generation accepts one terminal
  callback and fences reentrant adapter cleanup before scheduling exactly one
  retry. Production ESM and classic artifact scenarios now prove both pre- and
  post-acknowledgment persisted restoration under CSP in every supported browser.
- 2026-08-26 -- Staged initial and reconnect authorization, replay, event
  capability rotation, and authoritative-no-tail evidence behind the exact
  physical membership acknowledgment. Preflight cannot mutate island sequence
  or dispatch refresh, signal, or event effects. SSE control outcomes echo an
  opaque non-authority physical connection handle, control nonce, operation,
  subscription, stream, descriptor binding, and transport generation; a
  locally completed promise is not authentication. The first gap immediately
  fences the old generation and coalesces one recovery from the last committed
  position. Every scheduled physical handshake owns a bounded deadline that
  releases its per-origin permit even when the transport never opens.
- 2026-08-26 -- Required a typed, bounded, generation-fenced transport
  membership acknowledgment before replay or authoritative-no-tail evidence may
  prove physical continuity. SSE acknowledges only after its host control
  settles successfully; WebSocket uses a canonical post-commit frame bound to
  the exact control nonce, subscription, stream, descriptor binding, and
  document transport generation. The frame is minted only by consuming the
  exact commit's non-cloneable receipt, never by inspecting ambient membership.
  Rejection, timeout, close, and late/foreign/duplicate outcomes remain inert
  and cannot reset retries.
- 2026-08-25 -- Hardened the Task 6 browser boundary after adversarial review.
  Document transport groups now own compatible credential and aggregate retry
  authority; ordinary reconnect and bfcache restoration share bounded,
  deadline-owned current-position reauthorization; retry reset requires
  post-open continuity evidence; SSE controls and WebSocket membership frames
  have exact bounded ownership. Registered stream events use a core-minted
  opaque capability that snapshots full source/schema/scope/fanout/cycle
  authority, invalidates stale registrations, and accepts no caller fanout.
  Canonical payload ceilings count UTF-8 bytes, and the classic async artifact
  exposes one typed preboot configuration method while remaining inert by
  default.
- 2026-08-25 -- Bound authoritative refresh to the exact stored active
  authorization before continuity authority runs. Reconstructed caller
  authorization, including a substituted clock with otherwise matching signed
  facts, now fails through callback-free comparison without invoking continuity,
  clock, or registry callbacks and without changing sequence or pressure state.
  Final expiry and current-registry validation use only the stored authority.
- 2026-08-25 -- Closed replay's stored-authority, progress, and observability
  invariants. Replay now rejects foreign, detached, reconstructed-clock, and
  undeclared-payload input through bounded local validation before any host
  callback, then uses only the exact stored authorization and clock for its
  atomic current-registry seal. Recovery retains the effective required
  high-water even for a pressure-only obligation on a current sequence lane,
  and nested errors distinguish expiry, authorization loss, delivery retirement,
  and dispatcher failure while preserving the committed prefix. Every typed
  replay rejection increments the finite redacted `Rejected` counter exactly
  once; successful replay increments it zero times. Cancellation remains inside
  the closed synchronous document lifecycle rather than exposing a second public
  cancellation authority.
- 2026-08-25 -- Closed the final Task 5 replay and refresh invariants.
  Authoritative refresh now obtains a trusted proposed baseline before commit
  time and its final current-registry callback, then performs only callback-free
  validation/install and exact pressure reconciliation. Replay is accepted only
  for an existing exact recovery obligation; healthy lanes and malformed,
  noncontiguous, count, payload, or byte/capacity failures reject before host
  callbacks without manufacturing degradation. Post-prefix authorization,
  cancellation, and retirement retain truthful replay progress independently of
  the outer error, and replay closure starts once-only transport cleanup.
  `Coalesced` now means exact work was absorbed by the tail: redundant equal or
  lower work retains it, while exact-successor replacement degrades. MAX-tail
  comparison occurs before successor arithmetic, and resolved delivery remains
  both non-forgeable and non-cloneable.
- 2026-08-25 -- Closed the bounded-delivery authority surface after independent
  review. Raw Task 3 admission and sequence mutation remain private to the
  document owner; replay uses one atomic current-membership snapshot, rejects
  replayed lifecycle completion, preserves truthful dispatch prefixes, and
  exposes no host-callback gap after an entry's final validation. Registered
  dispatch consumes a private-construction resolved-delivery capability binding
  trusted target scope and fanout. Document-owned authoritative refresh covers
  exact sequence and pressure high-water; terminal buffers stop provider reads,
  extracted candidates retain RAII loss ownership, redundant tails create no
  false loss, and recovery idleness is document-owned. The unreachable
  `SequenceOverflow` branch was removed: at `u64::MAX`, same-epoch values are
  duplicates and only a newer epoch can advance through authoritative recovery.
- 2026-08-25 -- Bound Task 5 pressure recovery and terminal sequencing to exact
  logical memberships. The document retains finite redacted pressure causes by
  binding and scope, so one sibling's replay cannot clear another sibling's
  ordered overflow while its Task 3 lane remains current. Exact replay is still
  validated and committed by that existing Task 3 machine through the lost
  high-water; explicit authenticated retirement clears only its exact obligation.
  A queued terminal predecessor also fences exact and rotated same-ID admission
  until its retained lane drains, preventing duplicate sequence authority.
- 2026-08-25 -- Removed the remaining Task 5 delivery bypasses and preserved
  replay as recovery authority end to end. Raw Task 4 `next` is no longer a
  public API; provider delivery, replay admission, dequeue, and registered
  dispatch stay inside the bounded document owner. Commit/dispatch time now
  precedes final current-host validation with no later host callback. Replay
  count/capacity fail before allocation or validation, atomic queue members keep
  one private transcript boundary through lock-scoped dequeue, and the lease
  invokes `recover_from_replay`, clearing pressure degradation only after full
  success while retaining a truthful partial-failure outcome. Empty EOF drains
  and lanes are pruned immediately, including across exact-ID and rotated-wire
  reuse.
- 2026-08-25 -- Closed the final Task 5 authority-lifetime gaps. Raw sealed
  entries, buffer offer/replay, and delivery leases are private implementation
  values; the public document owner performs prevalidation, final current-host
  validation, exclusive commit-time expiry checking, and synchronous queue
  mutation without exposing a seal-to-offer window. It also owns one existing
  Task 3 sequence lane per exact logical binding and provides the only closed
  dequeue-and-dispatch operation. Its non-cloneable RAII lease holds the exact
  guard, shared permit/cancellation state, membership scope, and continuity
  tracker through registered dispatch; denial, failure, panic/drop, gaps, and
  cancellation degrade truthfully, while apply/duplicate/stale resolve without
  inventing currentness. Terminal lanes survive Task 4 detachment only through
  their bounded drain, and provider-failure purges degrade lost continuity.
- 2026-08-25 -- Hardened Task 5 around sealed current authority and one real
  document queue. `AsyncBackpressure` no longer accepts an envelope plus a
  caller-supplied fanout count; only the exact Task 4 document membership may
  mint an authorized entry after Task 3 current registry validation and trusted
  target resolution. One `BoundedDocumentTransportSession` composes Task 4 fair
  fan-in with the aggregate 64-entry/256-KiB queue and shared permits, without a
  staging buffer or second sequence machine. Shared queue batch admission,
  semantic tail replacement, and exact membership removal are atomic under one
  critical section; rejected values drop after unlock. Replay is same-scope and
  all-or-none, binding/scope enter coalescing identity, retirement purges only
  stale membership work, and graceful terminal drains preserve predecessors.
- 2026-08-25 -- Implemented server delivery pressure as a policy wrapper over
  the shared bounded-resource owner, queue, permits, and cancellation flag.
  Document queues retain at most 64 unapplied canonical envelopes or 256 KiB;
  payload, replay, and fanout are preflighted before delivery allocation, with
  event fanout capped by both current signed registration and deployment
  policy. Only an exact contiguous same-scope refresh or presentation-signal
  tail with the same registered schema contract may be replaced, and
  replacement truthfully marks continuity degraded.
  Ordered browser events and lifecycle records never coalesce or disappear;
  terminal close drains once, and observability has a fixed redaction-safe
  counter vocabulary.
- 2026-08-25 -- Split every external transport membership mutation into
  synchronous document snapshot, owned asynchronous authority/source work, and
  one-use synchronous commit. No document borrow crosses an await. Exact
  physical scope and owner, expiry, control generation, active-plus-retiring identity
  fences, and capacity are checked immediately before source work and commit;
  opened sessions retain once-only cleanup ownership through cancellation or a
  failed/stale commit. Pending controls have a hard RAII permit bound. Removal
  authenticates before local membership classification, preventing a WebSocket
  membership oracle.
- 2026-08-25 -- Bound logical membership to the exact signed-descriptor digest,
  including key ID and signature, rather than claims equality. Split physical
  `DocumentAuthorizationScope` from component-specific authorization memos so
  heterogeneous component contracts may share one connection only under exact
  principal/session/tenant/aggregate-policy identity. Replaced per-wake boxed
  read/close futures and inline close waits with persistent session polling and
  a bounded document-owned retirement lane. `Complete` now detaches before its
  single terminal delivery and suppresses all later source output; typed Error
  payloads remain nonterminal.
- 2026-08-25 -- Replaced retained transport admission with a fresh host
  authority port at every external add/remove boundary. Add validates both
  before and after asynchronous source subscription; authority loss, exclusive
  expiry, or registry/mode drift after the await closes the opened logical
  session once and commits no membership. Canonical registered modes now bind
  authority independently from the document compatibility kind, while internal
  retirement and shutdown remain able to clean up expired/revoked memberships.
- 2026-08-25 -- Implemented host-neutral cancellation-safe logical event
  sessions and one bounded document fan-in per compatible origin, transport,
  and authorization scope. The document layer routes only exact active
  subscription identities and never owns or mutates their independent Task 3
  sequence machines. SSE emits canonical bounded records with same-origin
  authenticated membership control around a correlation-only handle;
  WebSocket emits canonical bounded text/control frames and applies controls
  only with matching current descriptor authorization.
- 2026-08-25 -- Required a fresh non-cloneable active-membership guard for every
  sequence observation and made sequence application dispatch-aware. Exact-next
  position commits only after the closed registered dispatcher succeeds. Replay
  prevalidates its entire at-most-1,024-envelope transcript before work, commits
  each successful dispatch in order, and reports a truthful applied prefix,
  current position, degraded state, and retained high-water on first, middle,
  or final dispatch failure.
- 2026-08-25 -- Replaced claimed replay ranges with a bounded transcript of
  freshly membership- and registry-admitted envelopes. Gap recovery requires
  every same-scope, same-epoch position from the last applied successor through
  at least the recorded high-water with no empty proof, duplicate, regression,
  or omission. A new epoch or otherwise unavailable replay can be adopted only
  through the injected host continuity authority, and its baseline must not
  regress and must cover all observed high-water. Initial sequence authority is
  the baseline retained from the same verified Task 2 descriptor as the sealed
  membership context, never a caller-supplied machine argument.
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
