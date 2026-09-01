# Document work arbiter -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-08-25
Target domain: `11-interaction-scheduling-and-feedback.md`

## What it is

The Live browser runtime shall place one document-scoped network-work arbiter
above its independent per-island schedulers. Island schedulers continue to own
intent order, revision compatibility, cancellation, response application, and
feedback; the document arbiter owns only bounded admission to shared browser and
origin resources.

The arbiter shall provide one configured logical in-flight budget, explicit
capacity for asynchronous-transport establishment and recovery, aging priority
classes, fair service among islands with pending work, and one lifecycle source
for online, hidden, frozen, pagehide, pageshow, and shutdown state. It shall not
assume that browsers expose their physical connection limit or that HTTP/1.1,
HTTP/2, HTTP/3, SSE, and WebSocket consume identical native resources.

## Acceptance criteria

- Each document has exactly one network-work arbiter, while each Live island
  retains its existing scheduler and sequence/revision authority.
- The configured logical concurrency ceiling is bounded and never exceeds the
  framework's safe fallback for the active transport profile. A reserved lane
  keeps asynchronous connection establishment, control, heartbeat recovery, and
  reconnect work available during bursts of ordinary Live requests or uploads.
- Default admission classes are, from highest to lowest urgency: interactive
  application-user work; visible authoritative fresh render; background
  freshness work; and bulk upload chunks. Classes are framework metadata, not
  untrusted directive-selected security or authorization policy.
- Priority uses bounded aging or an equivalent minimum-service guarantee. A
  sustained stream of interactive work cannot permanently starve upload or
  background progress, and low-priority work cannot delay already-admitted
  interactive work by occupying an unbounded queue.
- Within each priority class, pending islands receive deterministic weighted
  round-robin or equivalent proven-fair service. One chatty island cannot own
  the document's admission capacity.
- Per-island FIFO, replacement, cancellation, latest-only, and safe-parallel
  policies remain authoritative after admission. The arbiter never reorders
  work inside an island contrary to that island's declared policy and never
  applies one island's outcome to another.
- Upload byte transfer continues to use its bounded data-plane queue, but new
  chunks obtain document admission. A chunk already committed to a provider is
  not described as canceled merely because higher-priority work arrives.
- Poll and push invalidations continue to become registered fresh-render
  intents in the owning island scheduler. Coalescing inside one island happens
  before document admission so superseded work consumes no permit.
- One document lifecycle authority decides whether each work class may admit,
  hold, cancel, back off, or retire while hidden, offline, frozen, entering
  bfcache, restored, or shutting down. Feature adapters do not maintain
  competing document pause state.
- Returning online or visible uses bounded jitter and fair admission; it cannot
  flush every island, poll, reconnect, and upload simultaneously.
- Navigation and native document requests remain browser-owned and are not
  converted into a Live client router merely to join this budget.
- Queue length, wait time, permit use, starvation prevention, suspension, and
  retirement are observable through bounded low-cardinality diagnostics.
- Deterministic tests use controlled clocks and barriers to prove priority,
  aging, fairness, reserved-capacity behavior, cancellation, lifecycle changes,
  and absence of starvation without sleeps.

## Touches

- Primary owner: `11-interaction-scheduling-and-feedback.md`.
- Runtime ownership and lifecycle: `09-runtime-bootstrap-and-directives.md`.
- Upload admission and data-plane interaction: `08-file-uploads.md`.
- Asynchronous transport, reconnect, polling, and document queues:
  `14-events-and-asynchronous-updates.md`.
- Architecture resource budgets and `D100`, `U4/16`, `E100/1K`, and `R100`:
  `00-overview.md` and `19-developer-tooling-and-testing.md`.
- Current compatibility constraint: per-island schedulers remain independent;
  the arbiter coordinates shared admission rather than creating document-wide
  component state, response ordering, or authority.
- Dependency: the current Iteration 004 upload and asynchronous managers must
  finish with their agreed bounded queues and lifecycle hooks before their work
  classes are placed behind one cross-feature arbiter.

## Why not now

This is a cross-feature redesign of existing action, refresh, upload, polling,
and reconnect admission rather than an implementation detail of Iteration 004
Task 4, so it is staged instead of being silently added to the active contract.
