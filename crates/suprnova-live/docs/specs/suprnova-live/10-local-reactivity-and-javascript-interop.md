# Suprnova Live -- 10 Local Reactivity and JavaScript Interop

Status: Normative design specification
Last revised: 2026-08-24

## Scope

This domain owns browser-local signals, derived local presentation, local
directives, signal lifetime across morphs, Stimulus controller integration, and
registered JavaScript/effect interoperation. It depends on runtime discovery
and constrains morphing. It does not own server-authoritative component state,
action transport, or a general-purpose client application framework.

## Capabilities

### Island-scoped local signals

Application developers shall be able to declare browser-owned state for UI
behavior that requires neither server authority nor server computation. Signals
shall be scoped beneath an island or explicit local root and shall never become
trusted domain, identity, authorization, or cache state.

Acceptance criteria:
- Signal declaration defines a name, supported value type, initial literal, and
  ownership scope without arbitrary expression evaluation.
- Duplicate, shadowed, and missing signal references have deterministic rules
  and checker diagnostics.
- Signals are inaccessible across islands unless a declared event interface
  communicates a value.
- Local changes make no network request by default.
- Sensitive values and durable domain records are rejected from the local-state
  contract.
- Server rendering determines the initial accessible markup and state
  attributes.

UX flow:
1. Application user toggles a disclosure or tab -> the local signal updates the
   DOM immediately without contacting Rust.
2. Behavior needs server authority -> the application uses a Live action rather
   than promoting the signal into client authority.

### Local presentation directives

The runtime shall provide bounded primitives for show/hide, toggle, class,
attribute, selected/expanded state, focus, and related presentation behavior.
They shall preserve semantic and accessibility state and compose with CSS
transitions without becoming a full template language.

Acceptance criteria:
- Each directive declares supported element types and signal value coercion.
- `hidden`, `aria-*`, focusability, and inert behavior remain consistent with
  visible state.
- Class and attribute changes use allowlisted names or safe literal metadata and
  cannot inject event-handler code.
- Initial server HTML does not flash a contradictory state before runtime
  connection when preventable.
- Reduced-motion preferences apply to local animations.
- Local behavior remains testable without a server round trip.

UX flow:
1. Signal changes -> dependent local directives apply one coherent DOM update
   and accessibility state.
2. Transition is unsupported or reduced -> final state still applies without
   motion.

### Signal lifetime and morph reconciliation

Local signals shall survive a Live morph only while their keyed ownership scope
survives. Server HTML remains authoritative over scope existence and may
explicitly reset local state; accidental DOM replacement shall not decide
signal lifetime.

Acceptance criteria:
- Stable keyed scopes retain compatible signal values across morphs.
- Removed, rekeyed, or explicitly reset scopes dispose their signals and
  subscriptions.
- Server changes to the declared initial value have a documented preserve,
  reset, or conflict rule.
- A document navigation resets document-scoped signals.
- Optional persistence requires a separate explicit storage key, schema,
  privacy, expiry, and migration contract.
- Morph failure cannot copy signals into an unrelated scope.

UX flow:
1. Server action rerenders around an open disclosure -> it stays open while the
   same keyed scope survives.
2. Application intentionally replaces that scope -> local state resets to the
   new server declaration.

### Stimulus controller integration

Stimulus shall be the supported substrate for application-specific browser
controllers that exceed Live's local primitives. Live and Stimulus shall have
defined DOM ownership and connect/disconnect behavior across morphs.

Stimulus remains application-supplied. Suprnova's bridge and continuity
implementation ships outside universal core as deterministic ESM/classic
adapter artifacts and a matching package export. The adapter registers before
the existing `boot({ stimulus: { application, definitions } })` call as a
singleton inside the closed lifecycle driver; it is not a third upload/async
feature slot. Missing or incompatible registration reports one bounded
unavailable diagnostic while local signals, server actions, and morphing remain
operational.

Acceptance criteria:
- Applications can attach standard `data-controller`, target, value, class, and
  action attributes in external templates.
- A preserved controller root remains connected without duplicate controller
  instances.
- Inserted and removed roots receive normal Stimulus lifecycle callbacks.
- Live exposes before/after morph and island lifecycle hooks without requiring
  controllers to patch the morph engine.
- Core owns validated ordering for before morph, successful after morph, morph
  abort, island retirement, suspend/resume, and document disposal. The optional
  adapter owns application validation, bounded definitions, controller-root
  scanning, continuity records, and `start`/`load`/`unload`/`stop`; its failure
  cannot veto validation, morph authorization, snapshot commit, or recovery.
- The public `StimulusApplicationPort`, `StimulusBootstrapOptions`,
  `StimulusContinuity`, and `StimulusMorphBridge` structural contracts and the
  ESM/classic boot behavior remain stable. Neither adapter imports or bundles
  `@hotwired/stimulus`.
- Controllers cannot mutate snapshot authority or mark an action accepted.
- Conflicting ownership of a protected DOM subtree is detectable and documented.

UX flow:
1. Application developer attaches a controller -> it connects through standard
   Stimulus semantics after initial SSR.
2. Live morph retains or removes its root -> controller continuity or disconnect
   follows explicit identity rather than wholesale replacement.

### Registered browser effects and JavaScript API

Server actions and components may request browser behavior only through named,
registered effect handlers with validated data. Application JavaScript may call
supported runtime APIs without gaining access to private scheduler or security
state.

Acceptance criteria:
- Built-in and application effect names register once with schema validation.
- Unknown effects fail safely and are observable.
- Effects cannot contain executable code strings or arbitrary module URLs.
- Effect timing relative to snapshot acceptance, morph, events, focus, and
  navigation is defined by the wire/runtime contracts.
- Public APIs cover action invocation, event dispatch, signal access within
  scope, lifecycle subscription, and approved diagnostics.
- API calls still pass through ownership, scheduling, and security checks.

UX flow:
1. Accepted action returns a registered effect -> runtime validates and invokes
   it at the defined phase.
2. Handler is absent or fails -> protected server work is not rolled back or
   represented ambiguously; the failure enters diagnostics and UI policy.

### Optimistic local projection boundary

Local reactivity may present a reversible optimistic projection while a server
action is pending, but it shall never declare authoritative success. Projection
and rollback behavior belongs to explicit interaction metadata and coordinates
with scheduling.

Acceptance criteria:
- Optimistic changes identify the action, affected local scope, committed state,
  and rollback behavior.
- Destructive, authorization-sensitive, or irreversible outcomes do not present
  false completion.
- Server success reconciles to returned HTML rather than trusting the projection.
- Rejection, timeout, or cancellation restores or refreshes from a known state.
- Accessibility feedback distinguishes pending from completed.

UX flow:
1. Application user invokes an eligible action -> local projection responds
   immediately and reports pending status.
2. Server accepts or rejects -> returned authoritative HTML confirms the result
   or rollback/recovery restores a truthful state.

## Acceptance criteria

- Local signals provide instant non-authoritative behavior without network
  requests.
- Signal scope and lifetime remain stable across compatible morphs.
- Local directives preserve semantic and accessible state.
- Stimulus and registered effects integrate through public lifecycle contracts.
- Optimistic projection cannot impersonate server-authoritative success.

## Decisions and revisions

- 2026-08-24 -- Moved Suprnova's optional Stimulus bridge and continuity state
  out of universal core and into one separately delivered lifecycle-driver
  singleton. Preserved the application-supplied port, unchanged boot options,
  ESM/classic support, keyed continuity, and exact cleanup semantics.
- 2026-08-21 -- Adopted two levels of reactivity: browser-local signals for
  presentation and Rust actions for authoritative work.
- 2026-08-21 -- Stimulus is the supported custom-controller substrate; rejected
  making it the Live protocol or state authority.
