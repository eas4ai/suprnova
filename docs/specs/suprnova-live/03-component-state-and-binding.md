# Suprnova Live -- 03 Component State and Binding

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns component-state categories, browser mutation permissions,
model binding, server-only and derived state, URL representation, and state-size
discipline. It depends on component metadata and feeds snapshots, actions, the
browser runtime, and navigation. Serialization envelopes, action execution, and
client scheduling belong to neighboring specs.

## Capabilities

### Explicit state categories

Every component field participating in Live shall have one unambiguous state
category. Ordinary component state is browser-immutable unless explicitly
exposed for model binding; sensitive, server-resident, session-backed, and
computed values shall not be conflated with client-carried state. A transient
model field shall accept a value for one request without becoming dehydrated
component state.

Acceptance criteria:
- Generated metadata identifies each field's category and wire representation.
- A field cannot simultaneously claim incompatible categories.
- Default component fields can round-trip in a signed snapshot but cannot be
  changed by a browser model update.
- Default component fields are instance-only state. Public-seed eligibility is
  an explicit separate declaration and never inferred from visibility,
  serialization support, or absence of a model binding.
- Locked fields reject browser mutation even if a payload names them.
- Secret material is excluded from browser-visible snapshots and HTML unless
  the application explicitly renders a safe derived value.
- Transient model values are accepted only in the current request envelope,
  remain redacted, and are never copied into the resulting snapshot.
- Unsupported field types fail checking with a path to a supported codec or
  server-only representation.

UX flow:
1. Application developer declares component fields -> generated metadata shows
   what crosses the browser boundary and what can be proposed by the browser.
2. Browser sends a mutation for a non-model field -> the update is rejected and
   no action observes the forged value.

### Transient model fields

Sensitive or request-scoped controls such as passwords, one-time codes, and
confirmation secrets shall be able to bind through an explicit transient model
category. The value exists only while processing the current request and shall
not survive through dehydration, diagnostics, events, effects, or cache data.

Acceptance criteria:
- Transient fields remain deny-by-default and require the same explicit binding
  metadata and typed conversion as ordinary model fields.
- A transient value is absent from every new instanced snapshot and public seed
  snapshot, including validation-failure outcomes.
- Logs, traces, diagnostics, fixtures, panic output, events, and browser effects
  redact the value by construction.
- Compatible validation morphs preserve the browser control's local value
  without requiring the server to echo it in HTML or snapshot state.
- An action that needs durable use passes the value directly into an authorized
  domain service; later requests receive no implicit copy.
- Transient fields cannot be URL-bound, session-backed, computed, or used as
  stable component identity.

UX flow:
1. Application user submits a transient secret -> the current action can
   validate and consume it without dehydrating it.
2. Validation fails or the request ends -> the browser may retain the local
   control value under morph policy, but no later server request receives it
   unless the application user submits it again.

### Model-bindable fields

Browser controls shall bind only to fields explicitly marked as model-bindable.
Binding shall preserve Rust types and distinguish missing, null, invalid, and
valid values before action logic depends on them.

Acceptance criteria:
- Binding paths are statically discoverable and validated against component
  metadata.
- Scalar, optional, collection, nested, enum, date/time, and identifier support
  is explicit rather than inferred from arbitrary JSON coercion.
- Invalid conversions produce field-level binding errors without panics.
- Unknown or forbidden paths are rejected as contract violations.
- Binding nested collections requires stable item identity where reordering can
  occur.
- HTML control semantics such as unchecked checkboxes and multi-select values
  map predictably to Rust values.

UX flow:
1. Application user edits a bound control -> the runtime proposes a typed value
   according to its declared timing.
2. Conversion fails -> the field exposes a binding error and the action cannot
   silently consume a fallback value.

### Server-only, session-backed, and computed state

Components shall reference sensitive or authoritative server data without
serializing that data into the snapshot. Session-backed values and computed
render results shall be reloaded or recomputed from their true authority on
each applicable request.

Acceptance criteria:
- Server-only state uses opaque identifiers or request context rather than
  browser-carried secret data.
- Session-backed fields read and write through Suprnova's session contract and
  participate in cache variance where rendered.
- Computed fields are not accepted from the browser and are not redundantly
  stored merely to avoid recomputation.
- Recomputed data is reauthorized before use.
- Missing or changed server data has explicit not-found, forbidden, or refresh
  behavior.

UX flow:
1. Live action requires authoritative data -> the server reloads it from the
   owning service using authorized identity.
2. Authority has changed since the previous render -> the component renders the
   current permitted state or an explicit recovery outcome.

### Binding timing declarations

Application developers shall declare when a model synchronizes using supported
timing policies such as action/submit, blur, change, debounce, or immediate.
Timing affects transport scheduling only and never expands mutation authority.

Acceptance criteria:
- Every modifier has one documented event and timing meaning.
- Debounce duration parsing and defaults are deterministic.
- Multiple timing modifiers that conflict fail checking.
- A final submit includes the latest allowed control values even when a prior
  debounce has not fired.
- Server and browser metadata agree on the field's binding permission.

UX flow:
1. Application developer selects a binding timing -> runtime feedback and
   request frequency follow that policy.
2. Application user submits before a scheduled sync -> the current permitted
   value is included once under the action scheduling contract.

### URL-bound state

Selected component state may be reflected in the current route's query string
when it describes shareable state, or it may select a real navigable document
state. Live reflection uses `history.replaceState` only: it creates no new
history entry and installs no `popstate` island router. State for which distinct
back/forward steps matter shall use ordinary document navigation.

Acceptance criteria:
- URL-bound fields declare encoding, default omission, validation, and either
  reflected or navigated behavior.
- Initial mount reads URL state through the router's typed parameter contract.
- Reflected state updates the current same-route query through
  `history.replaceState`, works after reload or sharing, and does not create an
  earlier Live state for Back to revisit.
- Navigated state uses a real route URL and normal document navigation whenever
  history entries or route/path changes are required.
- Sensitive, high-cardinality private, or non-serializable fields cannot bind to
  the URL accidentally.
- Back and forward retain normal browser document semantics and never invoke a
  hidden Live `popstate` action.

UX flow:
1. Application user changes reflected state -> Live morphs the island and
   replaces the current query URL; a history-significant change instead follows
   a real route link.
2. Another client opens the URL -> the route reconstructs the same shareable
   state subject to current authorization and data.

### State size and evolution

Component state shall remain bounded, versionable, and diagnosable. Large query
results, files, secrets, and durable domain records belong to their existing
server authorities rather than snapshots.

Acceptance criteria:
- Configurable per-field, per-snapshot, and per-request size limits fail safely.
- Diagnostics identify dominant state fields without exposing sensitive values.
- Schema evolution has an explicit compatibility or remount path.
- Collections intended for display can use identifiers, pagination, or server
  recomputation rather than unbounded snapshot copies.
- State encoding is deterministic enough for signing, revision checks, and
  tests.

UX flow:
1. Component approaches a configured state limit -> development diagnostics
   identify the design issue.
2. A request exceeds a hard limit -> the server rejects it before expensive
   hydration and offers the defined fresh-render recovery.

## Iteration 002 implementation profile

Iteration 002 implements generated field metadata and server execution for
ordinary instance state, explicitly public-seed state, model-bindable, locked,
transient, server-only, session-backed, computed, and secret categories. It adds
distinct `State` and `Session` metadata categories rather than treating default
state as public or hiding session access inside `ServerOnly`. It applies bounded
browser proposals through explicit typed codecs, keeps binding errors distinct
from validation errors, excludes every nondehydratable category from new
snapshots, and exposes session or authoritative data only through trusted host
contracts.

Promotion of a public seed reconstructs a fresh component through its registered
mount path before any action can observe it. The engine may then overlay only
verified `Public` seed fields. `State`, `Model`, `Locked`, `Session`,
`ServerOnly`, `Computed`, `Transient`, and `Secret` values come from current
mount defaults, trusted host capabilities, or the current typed proposal path -
never from missing seed fields or browser substitution. Mount preparation must
be safe to repeat and cannot perform external domain effects; mutations belong
to registered actions.

Binding timing and URL declarations are checked and emitted as metadata.
Initial typed query input and the server-side reflected/navigated URL decision
are testable through the host-neutral harness; browser events, debounce queues,
`history.replaceState`, and document navigation execution remain iteration 003.

## Acceptance criteria

- Browser mutation is deny-by-default and restricted to explicit model fields.
- State categories prevent secrets, computed data, and session authority from
  leaking into client-carried state.
- Binding preserves Rust types and reports invalid conversion safely.
- Timing and URL binding remain consistent with real routes and scheduling.
- State is bounded and evolvable without sticky server objects.

## Decisions and revisions

- 2026-08-21 -- Made ordinary component fields instance-only `State`, added a
  distinct nondehydrated `Session` category, and required explicit `Public`
  declaration for reusable seed state. Public-seed promotion freshly mounts the
  component before applying verified public values, so omitted categories cannot
  be reconstructed from browser input.
- 2026-08-21 -- Assigned all state categories, typed proposal application,
  transient-value redaction, session/computed host ports, and binding/URL
  metadata to iteration 002. Browser timing and URL application remain
  iteration 003.
- 2026-08-21 -- Component fields are browser-immutable unless explicitly
  declared model-bindable. Rejected public-field mass assignment.
- 2026-08-21 -- Computed results and sensitive server state remain outside the
  snapshot; session-backed values use the existing session authority.
- 2026-08-21 -- Added transient model fields for request-only secrets. Rejected
  dehydrating passwords, one-time codes, or equivalent values into snapshots.
- 2026-08-21 -- URL reflection uses `history.replaceState` only. State requiring
  distinct Back/Forward history uses real document navigation; rejected a
  `popstate`-driven island router.
