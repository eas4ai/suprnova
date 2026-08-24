# Suprnova Live -- 12 DOM Morphing and Identity

Status: Normative design specification
Last revised: 2026-08-24

## Scope

This domain owns bounded reconciliation of returned server HTML with an
existing Live island, including identity, nested ownership, form and focus
continuity, preservation controls, controller/signal continuity, animation
integration, and recovery. It depends on rendering, runtime lifecycle, local
reactivity, and scheduling. It does not own server rendering or whole-document
navigation.

## Capabilities

### Framework-owned morph contract

Suprnova shall define the public DOM reconciliation contract while using a
pinned, replaceable morph implementation behind an internal adapter. The
implementation shall never become the source of public identity semantics.

Acceptance criteria:
- Morph input is one validated current island root and one server-rendered
  replacement root for the server-accepted successor to its expected browser
  revision; the browser successor is not committed until morph success.
- The adapter exposes deterministic lifecycle hooks and operations needed by
  Live without leaking implementation-specific APIs.
- Conformance fixtures can run against an upgraded or replacement engine.
- Full-document or cross-island mutation is rejected.
- Returned scripts do not execute as an incidental morph side effect.
- Performance remains bounded for documented node/depth limits.

UX flow:
1. Runtime accepts a response -> it passes old and new matching island roots to
   the morph adapter.
2. Roots are incompatible or invalid -> no uncertain partial morph occurs and
   controlled recovery begins.

### Keyed identity and structural matching

Stable keys shall identify logical elements and component boundaries across
renders. Unkeyed matching may use safe structural rules but shall not guess
identity where reordering would lose application-user state.

Acceptance criteria:
- Key syntax, scope, uniqueness, and allowed value sources are explicit.
- Duplicate keys fail checking or morph validation with source-oriented
  diagnostics where possible.
- Keyed list reordering moves existing identity rather than recreating it.
- Changing a key intentionally creates new identity and disposes the old scope.
- Nested island roots are opaque independently owned keyed boundaries.
- Structural insertion/removal around conditionals is covered by adversarial
  conformance fixtures.

UX flow:
1. Server reorders a keyed list -> focused or locally stateful items retain the
   correct logical identity.
2. Application developer emits ambiguous or duplicate identity -> Live refuses
   silent state transfer and reports the contract violation.

### Form, focus, and selection preservation

Morphing shall preserve active browser interaction state when server output is
compatible, while still applying authoritative server changes deliberately.

Acceptance criteria:
- Active element, focus-visible state, text selection/range, scroll position
  where scoped, and composition input have defined preservation rules.
- Text, checkbox, radio, select, and multi-select controls distinguish current
  local edits from accepted server values.
- Server-requested reset or authoritative field correction is explicit.
- File inputs follow upload preservation and can never have local paths
  synthesized.
- Disabled, required, validity, ARIA, and other server attributes update without
  losing permitted local values.
- Focus recovery after removal targets an application-declared element or safe
  semantic fallback.

UX flow:
1. Application user edits or focuses a control during an action -> compatible
   returned HTML updates around it without stealing focus or reverting newer
   input.
2. Server intentionally invalidates/removes it -> focus and value follow the
   declared correction/removal policy.

### Preservation, ignore, replace, persist, and teleport controls

Application developers shall have explicit directives for subtrees that Live
must preserve, ignore internally, replace, persist across compatible ownership,
or render into a declared target. Each escape hatch shall define state,
security, accessibility, and cleanup behavior.

Acceptance criteria:
- Preserve and ignore are distinct: one retains selected state while the other
  assigns subtree DOM ownership elsewhere.
- Forced replacement disposes controllers, signals, uploads, and listeners
  before inserting new identity.
- Persisted elements declare a stable key and compatible source/target scope.
- Teleport targets are explicit, unique, authorized within the document, and
  retain logical island ownership.
- An ignored third-party widget cannot smuggle trusted snapshot or action state.
- Nested combinations that cannot be reconciled fail checking.

UX flow:
1. Third-party widget owns an ignored subtree -> Live updates its boundary
   contract without rewriting widget internals.
2. Application intentionally replaces or teleports content -> lifecycle,
   accessibility relationships, and focus move coherently.

### Signals and controller continuity

Morphing shall coordinate with local signal scopes and Stimulus lifecycle so
preserved identity retains behavior and removed identity disconnects cleanly.

Core owns the bounded, exactly-once ordering of validated before-morph,
after-successful-morph, abort, retirement, suspend/resume, and disposal events.
The optional Stimulus adapter owns controller-root scanning and continuity
records. Adapter callbacks run only after normal preflight and ownership checks,
cannot veto morph or response authority, and receive no usable stale port after
retirement.

Acceptance criteria:
- Keyed surviving signal roots retain compatible local values.
- Stimulus controller roots are not duplicated during a retained morph.
- Added and removed controllers receive connect/disconnect at defined phases.
- Before/after morph hooks cannot veto security validation or apply stale HTML.
- Extension errors are isolated and produce actionable diagnostics.
- Memory and observer leak tests cover repeated morph/remove cycles.

UX flow:
1. Morph retains a keyed controller and signal scope -> behavior continues
   without reinitialization flashes or lost local state.
2. Scope is removed -> controllers, effects, observers, and signals dispose
   exactly once.

### Transitions and animation

Island morphs may coordinate CSS and View Transition capabilities without
delaying authority indefinitely or hiding final accessible state. Motion shall
respect application-user preferences.

Acceptance criteria:
- Enter, leave, move, and state-change transitions have explicit start/end and
  cancellation semantics.
- Final server state applies even when animation APIs are unsupported or fail.
- Reduced-motion suppresses non-essential motion.
- Long or interrupted animation cannot leave controls in false disabled/loading
  state.
- Transition identity does not cross tenant, document, or unrelated island
  boundaries.
- Tests can disable time and animation nondeterminism.

UX flow:
1. Compatible morph includes a declared transition -> visual change animates
   while semantic state remains correct.
2. Motion is unavailable or reduced -> the same morph completes immediately.

### Controlled failure recovery

If validation or reconciliation fails, Live shall avoid knowingly partial DOM
application and recover through a fresh authorized island rendering when safe.
Wholesale replacement is reserved for explicit recovery or developer intent.
If the server has already consumed the successor revision, failure leaves the
browser on its prior snapshot and requires refresh rather than replay.

Acceptance criteria:
- Preflight validates root, identity, revision, and prohibited structures before
  mutation where possible.
- Failure reports whether no changes, rollback, or controlled replacement
  occurred.
- Recovery does not replay the prior action automatically.
- A post-acceptance morph failure never installs the returned snapshot over the
  old DOM; fresh rendering reconciles the browser's prior revision with the
  server's consumed successor.
- Unsaved input and active uploads are not silently reported as preserved if
  replacement loses them.
- Repeated morph/recovery failure disconnects the island and stops looping.
- Development diagnostics include a redacted DOM/identity explanation.

UX flow:
1. Morph cannot prove a safe reconciliation -> current DOM and browser snapshot
   stay when possible and recovery state becomes visible.
2. Fresh rendering is safe -> runtime replaces only that island deliberately or
   leaves it disconnected with an actionable error.

## Acceptance criteria

- Morphing is bounded to one accepted island and implementation details remain
  behind Suprnova's adapter.
- Keys and nested boundaries preserve logical identity under reorder and
  composition.
- Focus, selection, controls, uploads, signals, and controllers follow explicit
  continuity rules.
- Escape hatches and transitions cannot bypass ownership or accessibility.
- Failure uses controlled recovery rather than silent partial corruption.

## Decisions and revisions

- 2026-08-24 -- Separated core morph-event ordering from optional Stimulus
  continuity storage. Every former pending-continuity cleanup path now has an
  explicit abort, retire, or dispose edge; optional failure remains isolated.

- 2026-08-23 -- Iteration 004 makes active upload and stream resources part of
  explicit keyed continuity. A compatible surviving owner retains its
  current-document task; removal, replacement, or rekeying retires it exactly
  once and never transfers a native file, grant, connection, or sequence state
  to unrelated identity.
- 2026-08-21 -- Use pinned/vendored Idiomorph 0.7.4 behind a Suprnova
  abstraction; rejected writing a new DOM diff engine before a compelling need
  exists.
- 2026-08-21 -- Wholesale island replacement is a declared intent or recovery
  path, not the normal update model.
- 2026-08-21 -- A successful morph is the commit boundary for the returned
  browser snapshot and revision. Failure after server acceptance refreshes and
  never retries the original action.
