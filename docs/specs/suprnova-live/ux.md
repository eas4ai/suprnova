# Suprnova Live -- UX Specification

Status: Normative design specification
Last revised: 2026-08-21

<!-- How the user interacts with the software. This spec owns the map;
domain specs own the streets: per-feature flow detail lives in each
domain spec and defers to this document for how it fits the whole.
Not a visual design document. -->

## Interaction model

Suprnova Live has two connected interaction models. The application developer
authors and verifies a server-driven interface; the application user interacts
with the resulting canonical documents and Live islands. This specification
uses those qualified terms and avoids the ambiguous unqualified "user."

The application developer's primary loop is:

1. Define typed Rust component state, lifecycle behavior, validation, and an
   explicit registry of server actions.
2. Author an external HTML template that renders the component and declares
   Live directives, local behavior, stable identity, feedback states, and
   accessibility semantics.
3. Mount the component as a Live island within a real Suprnova route that
   returns a complete canonical document.
4. Choose deliberately between browser-local interaction and a
   server-authoritative Live action. Local behavior must not make a network
   request merely because it occurs inside a Live island.
5. Use contract checking and component tests to find invalid directives,
   inaccessible states, state-binding violations, and incorrect action
   behavior before deployment.
6. Observe and operate the route through normal Suprnova diagnostics,
   security, caching, and application-service integrations.

The application user's primary loop is:

1. Navigate to a real URL and receive meaningful server-rendered HTML,
   regardless of whether RenderCache supplied the complete response, server
   stitching composed it, or the route rendered it afresh.
2. When the Live browser runtime is available, each Live island connects at its
   existing DOM boundary without replacing the initial document.
3. Interact locally for behavior that needs no server authority, or invoke a
   Live action when application state or server computation is required.
4. See immediate, accessible feedback for dirty, queued, loading, validation,
   success, and error states as applicable to the interaction.
5. Receive a bounded DOM morph that preserves permitted browser state and does
   not disturb unrelated islands or document content.
6. Follow ordinary links, redirects, and form navigations to real routes.
   Optional prefetching and visual transitions may improve that navigation but
   must not replace it with client routing.

Stimulus controllers may add application-specific browser behavior inside or
outside a Live island. The Live browser runtime retains ownership of the Live
protocol, scheduling, directives, local signals, effects, and morph boundary;
controller code must integrate through defined lifecycle hooks rather than
competing for ownership of the same DOM.

## User journeys

### 1. Developer first contact: create a Live component

1. The application developer creates or scaffolds a Live component -> Suprnova
   provides the conventional Rust component and external template locations.
2. The developer declares typed state, model-bindable fields, validation, and
   registered actions -> generated metadata establishes the component's public
   Live contract and rejects arbitrary browser-selected method invocation.
3. The developer mounts the component in a real route -> the route renders a
   canonical document containing an independently identified Live island and
   its signed snapshot.
4. The developer runs the contract checker and tests -> invalid bindings,
   unknown actions, malformed ownership, and asserted behavior are reported in
   developer terms with source locations where available.

### 2. Application-user first contact: load a canonical document

1. The application user follows or enters a URL -> the browser performs normal
   document navigation to a Suprnova route.
2. Suprnova resolves the response -> a valid RenderCache entry may bypass route,
   ORM, and template work; otherwise the route renders and records its
   dependencies according to policy.
3. The browser receives the response -> complete HTML exposes the initial
   content before browser enhancement and contains no dependency on client
   rendering to reveal that content.
4. The Live runtime loads -> it connects instanced islands in place and prepares
   public seed-backed islands without creating server ledger state; both expose
   Live-dependent controls when compatible.

### 3. Perform a browser-local interaction

1. The application user opens a disclosure, toggles a menu, changes a tab, or
   performs similar non-authoritative behavior -> a local signal or browser
   controller responds immediately without a server request.
2. A later Live action morphs the surrounding island -> keyed surviving scopes
   retain their permitted local signals and controller continuity.
3. The application user leaves the document or the keyed scope is removed ->
   its local state resets unless the application explicitly owns persistence
   through a separate contract.

### 4. Edit and submit server-authoritative state

1. The application user changes a model-bound control -> the runtime marks the
   relevant state dirty and synchronizes it according to the declared timing,
   such as immediate, blur, debounce, or submit.
2. The application user invokes an action -> the runtime associates the action
   with its owning island, queues it, exposes loading state, and sends either the
   current instanced snapshot or a public seed plus proposed first-instance
   nonce, together with allowed input and action data.
3. The server verifies identity, integrity, seed promotion or expected base
   revision, authorization, binding, and action registration -> only a valid
   claimed request rehydrates the Rust component and executes the action.
4. Validation fails -> the response carries field and form errors, the island
   morph exposes them accessibly, entered values remain available, and focus is
   directed according to the form's declared policy.
5. The action succeeds -> a redirect navigates immediately; otherwise the
   runtime preflights and morphs before committing the new browser snapshot and
   revision, then reconciles validation/focus, dispatches events/effects, and
   clears completed feedback.

### 5. Continue through concurrent interaction

1. The application user triggers multiple interactions in one island -> the
   runtime applies that island's queue, cancellation, or concurrency policy
   rather than allowing responses to race into the DOM.
2. Another island is active at the same time -> it proceeds independently and
   does not enter the first island's queue.
3. A response arrives for an obsolete island revision -> the runtime does not
   apply stale HTML over newer state and follows the defined resynchronization
   path when ordering alone cannot recover.
4. The accepted response morphs -> focus, text selection, form controls, keyed
   children, local signals, controllers, and transition state are preserved
   wherever their explicit contracts allow.
5. The server accepted but the browser morph fails -> the prior browser snapshot
   remains, the island fresh-renders, and the original action is never replayed.

### 6. Navigate to another document

1. The application user follows a normal link, submits a navigation form, or
   receives a server-directed redirect -> the browser navigates to the target
   Suprnova route.
2. Prefetching is enabled and safe for the target -> it may reduce latency but
   does not change route authority, history, URL, or response semantics.
3. View transitions are supported and motion is permitted -> the old and new
   documents may participate in a visual transition using stable transition
   identities.
4. Enhancement is unsupported or disabled -> navigation still completes as a
   normal document load without the visual enhancement.

### 7. Receive server-pushed change

1. An island declares an authorized broadcast subscription -> Suprnova's
   existing real-time infrastructure establishes it separately from the HTTP
   action transport.
2. A matching server event arrives -> the island performs only the declared
   refresh or action behavior through the same authority and morph contracts as
   an ordinary interaction.
3. The push connection drops -> HTTP actions and document navigation continue;
   the runtime reconnects according to policy and surfaces loss of freshness
   when the application experience depends on real-time delivery.

### 8. Add custom browser behavior

1. The application developer attaches a Stimulus controller to semantic HTML ->
   Stimulus owns the custom behavior but not the Live transport or morph.
2. A Live morph retains the controller's keyed element -> controller continuity
   follows the integration lifecycle rather than depending on accidental DOM
   replacement behavior.
3. The element is intentionally removed or replaced -> the controller receives
   a defined disconnect and any newly inserted controller receives a defined
   connect.

## Surface map

### Application-developer surfaces

| Surface | Entry point | Available actions | Owns |
|---|---|---|---|
| Rust Live component | Component module and derives/attributes | Mount, declare state, validate, authorize, register actions, render, dispatch | Server-authoritative component behavior |
| External HTML template | Component view file | Render HTML, bind models, invoke actions, declare local behavior, keys, feedback, preservation, transitions | Markup and presentation interaction contract |
| Suprnova route | Router and normal HTTP handler | Return canonical document, mount islands, select cache and variance policy | Document identity and HTTP semantics |
| RenderCache policy | Route or rendering configuration | Opt in, vary, declare or capture dependencies, select private/shared composition and coherence | Representation reuse contract |
| Contract checker | Build, check, and development workflow | Validate component/template/directive relationships and report diagnostics | Static or build-time contract feedback |
| Live test harness | Rust test suite | Mount, set allowed models, call actions, assert HTML, errors, events, redirects, snapshots, and authorization | Deterministic component interaction verification |
| Browser controller | Stimulus controller and lifecycle hooks | Add custom client behavior and integrate with morph lifecycle | Application-specific browser behavior |
| Component library | Catalog, templates, and theme tokens | Select, compose, customize, and test accessible Live-aware components | Official reusable interaction patterns |

### Application-user surfaces

| Surface | Entry point | Available actions | Required states |
|---|---|---|---|
| Canonical document | Real URL | Read, follow links, submit ordinary forms, enter Live islands | Loading through browser navigation, content, empty, HTTP error |
| Live island | Server-rendered island root | Interact with owned controls and receive targeted updates | Connecting when material, ready, queued, loading, success, error |
| Model-bound form | Semantic form controls | Edit, blur, submit, reset where offered | Clean, dirty, validating where applicable, invalid, submitting, success |
| Local interaction scope | Local directive or controller root | Toggle, show, hide, select, focus, animate | Current local value and any accessible expanded/selected state |
| Validation feedback | Field or form error region | Review errors, correct input, resubmit | Absent or present; associated with the affected controls |
| Navigation control | Link, navigation form, or redirect result | Navigate, cancel where the browser permits, traverse history | Native browser states plus optional prefetch and transition feedback |
| Real-time region | Authorized broadcast-backed island | Observe update, invoke ordinary Live actions, recover connection | Current, reconnecting, stale when material, error |

New features must attach to one of these surfaces or revise this map
explicitly. Domain specs own the detailed actions and acceptance criteria for
their surface; they must not invent a second navigation, transport, or state
ownership model.

## Decision points and branching

1. **Frontend mode:** a route or rendered surface is traditional Suprnova SSR,
   Suprnova Live, or Inertia. A Live surface does not embed an Inertia rendering
   protocol, and an Inertia surface does not assume ownership of Live islands.
2. **Local or server interaction:** behavior with no server authority or
   computation remains local; behavior that reads or changes authoritative
   state invokes a registered Live action.
3. **Model synchronization timing:** the application developer chooses a timing
   supported by the binding contract. The choice changes request timing, not
   which fields the browser is allowed to propose.
4. **URL behavior:** Live may reflect same-route query state into the current
   history entry with `replaceState`; state needing distinct Back/Forward steps
   uses real document navigation.
5. **Cache composition:** public reusable content may enter a shared
   representation; request-specific or identity-bound content uses explicit
   variance, a private representation, server stitching, or bypass according to
   its coherence and privacy requirements.
6. **Action result:** an accepted action may return an island morph plus a new
   snapshot, validation errors, declared browser effects, events, or a real
   redirect. The protocol defines their application order.
7. **Nested ownership:** an interaction belongs to the nearest owning Live
   island unless an explicit parent/child event contract says otherwise. A
   parent morph must not silently erase an independently owned child.
8. **No JavaScript:** the canonical document remains meaningful. Live-dependent
   controls remain unenhanced and do not acquire synthesized HTTP behavior; an
   application that requires equivalent interaction provides an ordinary route,
   link, or form explicitly.
9. **Enhancement capability:** prefetch, view transitions, real-time delivery,
   and richer browser behavior are used only when supported and permitted. Core
   document and HTTP semantics do not branch.

## Error and recovery flows

| Failure | Application-user experience | Recovery contract |
|---|---|---|
| Canonical route or render failure | The normal Suprnova HTTP error surface appears; Live does not start for a document that was not produced | Standard route error handling, retry, or navigation owns recovery |
| Live runtime unavailable, blocked, or incompatible | Initial server-rendered content remains exposed; Live-dependent controls do not pretend to have succeeded | Restore a compatible runtime or reload after deployment/configuration correction; no alternate action transport is synthesized |
| Network interruption or action timeout | The current DOM and last accepted snapshot remain; pending feedback changes to an accessible interrupted or error state | Retry only under the action's idempotency and scheduling rules; otherwise require a deliberate application-user retry |
| Validation failure | Entered values remain visible and associated errors appear without document navigation | Correct the identified values and submit again |
| Authentication, authorization, or CSRF rejection | No protected action effect is applied; feedback reveals no sensitive state | Follow the application's sign-in, permission, refresh, or navigation path |
| Invalid, expired, tampered, identity-mismatched, promotion-rejected, ledger-missing, or protocol-incompatible snapshot | The action is not executed and untrusted HTML or state is not applied | Request a fresh authorized rendering of the island or canonical document; do not silently reinterpret the rejected snapshot |
| Duplicate, stale, or out-of-order response | Older output never overwrites a newer accepted island revision | Ignore a provably obsolete response; otherwise obtain a fresh island rendering through the defined resynchronization path |
| Server action exception | Existing island content remains when safe, pending state clears, and an accessible error is exposed | Retry, correct input, navigate, or obtain a fresh rendering according to the owning action's error contract |
| Morph or identity-contract failure | The runtime does not leave a knowingly partial reconciliation in place | Emit an actionable developer diagnostic and perform a controlled fresh island rendering when safe; replacement is a recovery path, not the normal update model |
| Real-time connection loss | Ordinary HTTP actions and navigation remain available; freshness loss is shown when material | Reconnect according to policy and refresh affected islands when continuity cannot be proven |
| Cache rebuild failure | A policy-permitted stale representation may be served without changing interaction semantics; otherwise the route's error surface appears | The coherence policy determines stale serving, singleflight rebuild, retry, and failure escalation |
| Custom controller failure | Failure remains scoped to the custom behavior where isolation is possible | Report through developer diagnostics; the Live runtime must not treat arbitrary controller state as server authority |

Success feedback must be proportional to the action. A morph that makes the
result self-evident need not add noise; destructive, delayed, redirected, or
otherwise ambiguous actions must expose confirmation through an accessible
surface. Empty states must explain the absence of content and provide a next
action when one exists rather than appearing as a rendering failure.

## Platform divergences

- **JavaScript unavailable:** the application user receives the same initial
  canonical content, but Live directives, actions, synchronization, signals,
  effects, and morphing do not run. Explicit ordinary Suprnova interactions are
  the only no-JavaScript action path.
- **Non-browser HTTP consumers:** clients, crawlers, preview generators, and
  agents receive the canonical HTML representation and normal HTTP metadata;
  they are not required to implement the Live protocol to understand initial
  content.
- **View-transition support:** supporting browsers may animate document and
  island changes. Unsupported browsers receive the same state change without
  the transition.
- **Reduced-motion preference:** non-essential motion is suppressed or reduced
  without changing information, state, or available actions.
- **Keyboard and assistive technology:** semantic controls, focus order,
  accessible names, state attributes, error associations, and status
  announcements must expose the same interaction and outcome available to
  pointer and visual use.
- **Touch, pointer, and viewport differences:** components may change layout and
  presentation, but they retain the same ownership, action, validation, and
  recovery semantics.
- **Intermittent connectivity:** local interactions continue; server actions
  expose interruption and follow their retry contract. The interface must not
  report an authoritative success before the server has accepted it.
- **Real-time transport unavailable:** ordinary HTTP Live actions remain the
  foundation. Features that require current pushed state expose degraded
  freshness rather than silently presenting stale information as current.
- **Multiple tabs or restored documents:** each document instance owns its
  local signals, island queues, and last accepted snapshots. Server
  authorization and revision checks remain authoritative when another context
  has changed the underlying domain state.

## Decisions and revisions

- 2026-08-21 -- Defined the UX as two connected experiences: application
  developer authoring and application-user interaction. Rejected a UX limited
  to application users because framework ergonomics and diagnostics are part of
  the product.
- 2026-08-21 -- Normal document navigation remains the only navigation model.
  Prefetching and transitions are enhancements, not a client router.
- 2026-08-21 -- Initial HTML remains meaningful without JavaScript, while Live
  actions require the Live runtime. Rejected generated no-JavaScript action
  parity because it would duplicate transport complexity.
- 2026-08-21 -- Local signals and custom browser behavior handle non-authoritative
  interaction; registered Rust actions handle server-authoritative work.
- 2026-08-21 -- Public cached islands promote signed seeds on first action;
  non-redirect responses commit browser snapshot/revision only after morph.
- 2026-08-21 -- URL reflection uses same-route `replaceState` only, while
  history-significant state remains ordinary document navigation.
