# Suprnova Live -- 11 Interaction Scheduling and Feedback

Status: Normative design specification
Last revised: 2026-09-01

## Scope

This domain owns per-island request scheduling, model-update coalescing,
concurrency and cancellation policies, response ordering, retry/offline
behavior, and application-user feedback states. It depends on state binding,
wire transport, runtime directives, and local optimistic projections and feeds
morphing. Server idempotency and snapshot revision authority remain owned by
their server specs.

## Capabilities

### Per-island scheduling

Every connected island shall have an independent scheduler. The safe default
shall serialize operations in intent order; application developers may select
another documented policy only where action semantics can tolerate it.

Acceptance criteria:
- Default FIFO scheduling prevents two responses from racing into one island.
- Separate islands proceed independently unless an explicit application-level
  coordination contract joins them.
- Policies cover queue, replace/cancel pending, drop duplicate, latest-only, and
  explicitly safe parallel work.
- A policy declares whether it can cancel unsent work, in-flight transport, or
  only response application.
- Removing or disconnecting an island retires its scheduler without applying
  orphaned responses.
- Queue length and wait time are bounded and observable.
- Poll ticks and push invalidations queue registered refresh intents through the
  same island scheduler and coalesce under their declared freshness policy.
  Upload byte transfer uses its bounded data-plane queue and schedules only
  authoritative Live state work through the island scheduler.
- A fresh-render producer may attach one bounded completion observer to its
  scheduler intent. That observer settles exactly once from the scheduler's
  terminal application disposition (`succeeded`, `failed`, `canceled`, or
  `retired`); queue admission is never reported as HTTP/protocol success and the
  observer cannot mutate scheduler authority.

UX flow:
1. Application user triggers repeated actions -> the owning policy orders,
   coalesces, cancels, or rejects them predictably.
2. Another island is active -> it remains responsive under its own queue.

### Model update coalescing

Model updates shall honor action/submit, blur, change, debounce, throttle, and
immediate timing while coalescing superseded values safely. An action shall see
the latest allowed control state required by its form semantics.

Acceptance criteria:
- Debounce and throttle timers are scoped to field, directive target, and island.
- A submit flushes or incorporates pending allowed updates exactly once.
- Superseded unsent values do not create needless requests.
- An in-flight update response cannot overwrite a later local edit.
- Dirty state compares current browser proposal to the last accepted
  server-authoritative value.
- File inputs follow the upload contract rather than ordinary JSON coalescing.

UX flow:
1. Application user types into a debounced field -> local value and dirty state
   update immediately while transport waits.
2. Application user submits before the timer -> the action receives the latest permitted
   value without a duplicate stale update afterward.

### Targeted feedback states

The runtime shall expose declarative idle, dirty, queued, loading, uploading,
validating, success, interrupted, offline, retrying, and error states scoped to
an island, action, field, or explicit target. Feedback shall be accessible and
must not imply completion before server acceptance.

Acceptance criteria:
- Feedback directives can target one action/model or aggregate compatible work.
- Disabled, busy, and status semantics remain consistent and keyboard safe.
- Loading indicators avoid flicker through documented delay/minimum-duration
  options without hiding material latency.
- Success state has a defined duration or server-reset rule.
- Validation and transport errors remain distinguishable to diagnostics while
  presentation can share components.
- Status announcements avoid duplicate or excessively frequent live-region
  output.

UX flow:
1. Work enters the queue and transport -> relevant controls and status regions
   expose truthful queued/loading state.
2. Work settles -> feedback changes to success, validation, interrupted, or
   error and returns to idle under policy.

### Response ordering and stale suppression

Only a response compatible with the island's current request, revision, and
local edit state shall be eligible to advance its accepted browser state. A
non-redirect response shall not install its successor snapshot or browser
revision until its required morph, or explicit no-render preflight, succeeds.
Stale or mismatched HTML shall never be morphed merely because it arrived last.

Acceptance criteria:
- Correlation and revision checks precede morph and effect application.
- A redirect or protocol-v2 `navigated` URL intent is terminal and performs real
  navigation without first morphing, committing browser state, scheduling child
  delivery, or dispatching response events/effects.
- Non-redirect application follows protocol order: validate and preflight,
  morph, commit browser snapshot/revision, reconcile model/validation and focus,
  queue signed child deliveries and apply same-route URL reflection, dispatch
  events, run effects, then settle feedback. Child operations enter their own
  scheduler and are not atomic with the accepted parent morph.
- Only after that parent commit, each validated delivery is paired with the
  exact accepted top-level parent snapshot and queued as one ordinary child
  `params_changed` intent. The request sends the child's current snapshot and
  the exact v2 admission carrier, never raw parameters. Redirect, malformed
  response, failed morph, stale/mismatched child boundary, unchanged hash, or
  removed child schedules nothing.
- Parameter coalescing is scoped to one child incarnation. The current applied
  value is identified by hash, while pending work is identified by the exact
  canonical envelope-plus-parent-snapshot authority. Enqueue never marks a hash
  applied: parent revisions N and N+1 may queue the same hash, only exact pending
  authority coalesces, and the hash becomes current solely after accepted child
  application. Failure releases only its own authority for retry, and a later
  A -> B -> A change remains schedulable.
- Child scheduling runs after the accepted parent's rollback boundary. A single
  child's intent construction or enqueue failure is contained to that delivery,
  cannot recover or roll back the parent, and does not suppress other validated
  deliveries.
- Canceled, superseded, duplicate, and out-of-order outcomes have distinct
  handling.
- A response can update accepted server state without overwriting a newer
  unsent local edit.
- Morph or order failure after server acceptance keeps the previous browser
  snapshot and triggers bounded fresh-render recovery without retrying the
  original action.
- Navigation or island removal prevents late response application.
- Ordering tests use adversarial latency, cancellation, and duplicate delivery.

UX flow:
1. Responses arrive out of network order -> scheduler accepts only the result
   valid for current island intent.
2. Validity or morph success cannot be proven -> current DOM and browser
   snapshot remain and the island obtains fresh authoritative state.

### Interruption, offline, and retry

Network interruption shall preserve the last accepted DOM and snapshot, surface
truthful state, and retry only when transport and action idempotency permit it.
Local interactions shall continue while their scope remains available.

Acceptance criteria:
- Offline detection is advisory and actual request failure remains authoritative.
- Pending requests transition to interrupted or offline without false success.
- Safe automatic retry uses bounded exponential backoff, jitter, attempt limits,
  and the same idempotency identity.
- Non-idempotent or uncertain actions require deliberate recovery rather than
  automatic duplication.
- Returning online does not flush obsolete queued work over newer state.
- Application developers can supply scoped retry, discard, refresh, or navigate
  controls.

UX flow:
1. Connection fails during an action -> current content remains and the island
   exposes interrupted/offline feedback.
2. Recovery becomes safe -> runtime retries under idempotency policy or asks the
   application user to retry, discard, or refresh.

### Cancellation and navigation coordination

Cancellation shall distinguish local intent cancellation from server-side
effect rollback. Document navigation, island removal, and user cancellation
shall stop future browser application without promising to undo work already
accepted by the server.

Acceptance criteria:
- UI text does not label an uncertain in-flight effect as rolled back.
- Abortable transport uses standard browser cancellation where supported.
- Server action cancellation is cooperative and explicit when offered.
- Navigation guards account for dirty state and active uploads without turning
  navigation into a client router.
- Late accepted server effects remain observable through subsequent fresh state.

UX flow:
1. Application user cancels queued work -> it is removed before transport and
   feedback returns to the prior state.
2. Application user leaves during in-flight work -> response application stops;
   subsequent
   navigation reflects whatever the server actually committed.

## Acceptance criteria

- Each island owns one bounded scheduler with safe serialized defaults.
- Model timing and submit behavior cannot lose the latest permitted input.
- Feedback states are truthful, targetable, and accessible.
- Stale, duplicate, canceled, and out-of-order responses cannot overwrite newer
  intent.
- Retry and cancellation never imply rollback or duplicate effects without
  proof.

## Decisions and revisions

- 2026-09-01 -- Completed post-morph child pairing through the existing
  per-island scheduler. The accepted parent snapshot is paired at queue time,
  not duplicated in response deliveries; child coalescing, ordering, feedback,
  and recovery remain ordinary scheduler behavior and cannot roll back the
  accepted parent.
- 2026-08-26 -- Bound poll failure policy to the existing scheduler intent's
  actual terminal application disposition rather than queue admission. The
  fresh-render port now carries one optional, isolated completion observer;
  scheduler overlap remains one in-flight plus one queued refresh with no
  second queue, transport, or timer owner.
- 2026-08-23 -- Integrated Iteration 004 without creating competing action
  schedulers: poll and push refreshes enter the existing per-island queue,
  invalidation bursts coalesce under freshness policy, and upload byte transfer
  uses a separate bounded data-plane queue while its authoritative component
  outcomes retain normal Live ordering.
- 2026-08-22 -- Completed protocol-v2 browser ordering for iteration 003:
  navigated URL intent is terminal like redirect; after a committed non-redirect
  parent outcome, signed child deliveries are queued and same-route URL
  reflection applies after model/validation/focus reconciliation but before
  events and effects. Child application remains independent and non-atomic.
- 2026-08-21 -- Per-island serialized scheduling is the safe default; explicit
  policies may opt into cancellation, coalescing, or parallelism.
- 2026-08-21 -- Offline and optimistic behavior remain truthful projections, not
  client authority.
- 2026-08-21 -- Browser state commits only after successful morph or validated
  no-render handling. Redirect is terminal; post-acceptance failure refreshes
  and never replays the original action.
