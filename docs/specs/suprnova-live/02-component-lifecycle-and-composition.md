# Suprnova Live -- 02 Component Lifecycle and Composition

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns what a Live component is, how it is registered, mounted,
rendered, reconstituted for an interaction, composed, nested, and retired. It
depends on views and documents and feeds state binding, actions, snapshots, and
DOM identity. It does not own the wire envelope, browser scheduling, or morph
algorithm.

## Capabilities

### Typed component definition and registration

Suprnova shall provide a Rust-native component definition with generated
metadata for its stable name, view, state schema, actions, bindings, lifecycle,
and child relationships. Registration shall be explicit and closed to
browser-selected arbitrary Rust methods or types.

Acceptance criteria:
- A component can be defined without hand-writing serialization or action
  dispatch boilerplate.
- Component identity is stable across builds according to an explicit naming
  and versioning rule.
- Duplicate names and incompatible registrations fail during development or
  startup.
- Only registered components may be mounted or rehydrated.
- Generated metadata is consumable by the checker, test harness, and runtime.

UX flow:
1. Application developer defines and registers a component -> Suprnova exposes
   its generated contract to rendering and tooling.
2. Registration conflicts or lacks required metadata -> startup or checking
   fails with the conflicting source locations.

### Mount, render, and lifecycle ordering

A component shall have deterministic lifecycle phases for initial mounting,
rehydration, allowed model application, action execution, validation, rendering,
dehydration, and teardown. The programming model may feel stateful while each
ordinary action request remains reconstructible without a persistent server
object.

Acceptance criteria:
- Initial mount and action rehydration are distinct lifecycle paths.
- Every hook has documented ordering, allowed mutations, sync/async behavior,
  and failure semantics.
- Mount receives only authorized context and explicit parameters.
- An action request reconstructs the component from validated inputs rather
  than locating a sticky in-memory instance.
- Teardown and cleanup hooks run where resources were actually acquired and do
  not pretend to provide durable cross-request memory.
- Lifecycle failures stop later phases that rely on their result.

UX flow:
1. Initial document mounts a component -> mount initializes state and render
   produces the first island.
2. A later action arrives -> Suprnova rehydrates, applies allowed changes, runs
   the action and hooks, renders, and dehydrates a new result.

### Component parameters and stable identity

Parent views and components shall pass typed mount parameters into a child while
stable keys distinguish logical instances. Parameters initialize or refresh a
child according to explicit rules; they do not grant the browser permission to
mutate internal state. When a surviving independent child receives changed
parameters from a parent action, the parent response shall carry a signed
parameter-update envelope and a comparable parameter hash at the preserved
child boundary.

Acceptance criteria:
- Parameters are typed, validated, and attributable to their server render
  source.
- Repeated components in a list require stable developer-supplied keys when
  position is not identity.
- A key collision within an ownership scope is detected and diagnosed.
- Parameter change behavior distinguishes remount, update, and no-op.
- A child-parameter update envelope is bound to the issuing parent instance,
  accepted parent revision, child key, child component contract, parameter
  schema, and parameter value hash.
- After the parent morph, a changed hash queues one `params_changed` operation
  through the child's ordinary scheduler and revision-bearing protocol path.
- Browser-supplied raw parameters cannot substitute for the signed envelope.
- The server verifies that the issuing parent revision remains an eligible
  source and rejects a replayed envelope superseded by a later accepted parent
  revision; the child records/order-checks the applied parent revision.
- Keys never contain secrets and are not treated as authorization evidence.

UX flow:
1. Application developer renders keyed child components -> each logical child
   retains a stable lifecycle identity.
2. Parent parameters change for a surviving child -> the parent morph preserves
   the child DOM and the runtime schedules its signed parameter update.

### Nested component ownership

Nested Live components shall be independently owned islands when declared as
such. Parent rendering may position children and pass parameters but shall not
silently absorb, duplicate, or destroy a child's state and request queue.

Acceptance criteria:
- The nearest declared island boundary owns an interaction.
- Parent and child snapshots, revisions, and browser queues remain distinct.
- Parent morphs preserve a surviving keyed child boundary without rerendering
  the child as ordinary markup while updating permitted boundary metadata.
- Parent-child communication uses declared parameters or events.
- Parameter propagation is not atomic with the parent morph: the child exposes
  a bounded pending state, orders the update against its own work, and recovers
  only the child if the update fails.
- Removal of a child retires its browser and server-side ephemeral resources.
- Circular composition is detected before unbounded rendering.

UX flow:
1. Application user acts in a child -> only the child action and queue execute.
2. Parent output changes around the child -> the keyed child survives or is
   intentionally removed; changed signed parameters settle through the child
   scheduler without rolling back the accepted parent morph.

### Lazy and conditional components

Application developers shall be able to conditionally mount or lazily complete
expensive components without weakening canonical-document semantics. A lazy
boundary shall render explicit initial HTML and declare how and when completion
occurs.

Acceptance criteria:
- Conditional omission and lazy completion are distinguishable.
- A lazy boundary has semantic placeholder, loading, empty, error, and success
  behavior appropriate to its content.
- Lazy completion uses the normal Live authority, snapshot, scheduling, and
  morph contracts.
- Content essential to the canonical document's meaning is not hidden solely
  behind a mandatory client render.
- Lazy work can be disabled or eagerly executed in tests and non-browser
  contexts.

UX flow:
1. Application user receives a document with a lazy island -> meaningful
   initial markup explains or represents the pending region.
2. Runtime requests completion -> the island transitions through loading to
   rendered, empty, or error state without disturbing the document.

## Acceptance criteria

- Components have explicit generated registration and deterministic lifecycle
  ordering.
- No ordinary interaction depends on sticky server component instances.
- Typed parameters, keys, nesting, and removal have unambiguous ownership.
- Lazy components preserve meaningful SSR and normal Live recovery contracts.
- Component metadata supports downstream checking, testing, and protocol work.

## Decisions and revisions

- 2026-08-21 -- Adopted snapshot-reconstructed stateful semantics rather than
  persistent server component objects.
- 2026-08-21 -- Nested components are independently owned islands, not incidental
  replaceable fragments.
- 2026-08-21 -- Reactive parent-to-child parameters use signed envelopes bound
  to parent revision and child key. Delivery is scheduled and non-atomic with
  the parent morph; rejected raw browser-forwarded parameters.
