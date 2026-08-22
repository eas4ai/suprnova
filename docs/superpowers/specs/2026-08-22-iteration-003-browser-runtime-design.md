# Iteration 003 Browser Runtime Design

Status: Approved design
Approved: 2026-08-22

## Goal

Iteration 003 turns the existing TypeScript conformance package into the
production Suprnova Live browser runtime. It connects server-rendered Live
islands in place, keeps non-authoritative interaction local, transports and
orders server-authoritative work, reconciles accepted island HTML without
losing permitted browser state, and enhances real document navigation without
creating a client router.

The runtime implements the complete contracts in specifications 09 through 13.
It remains inside the dedicated `suprnova-live` development workspace. The
standalone build proves a host-neutral asset and browser contract; it does not
claim that Suprnova currently serves or registers those assets.

## Chosen approach

Iteration 003 remains one numbered scope contract with working vertical
milestones. Splitting bootstrap, signals, scheduling, morphing, and navigation
into separately shippable numbered iterations would create artificial partial
products and compatibility boundaries. Building a runtime shell first and
calling richer behavior later would violate the project's complete-not-MVP
rule.

The implementation therefore progresses through end-to-end capabilities:

1. load one deterministic artifact and connect a valid SSR island;
2. perform one local interaction without a request;
3. send and apply one server-authoritative action through the existing protocol;
4. preserve keyed identity, focus, form state, signals, and controllers through
   a successful morph;
5. complete bounded scheduling, recovery, extension, lifecycle, and navigation
   behavior across multiple islands and adversarial browser conditions.

Each milestone is independently tested, but none redefines the iteration's
completion boundary.

## Architecture

The production runtime is a strict-TypeScript package rooted at `browser/`.
Existing canonical, protocol, ordering, and conformance modules remain the
trusted validation foundation. New modules are grouped by owned behavior rather
than by a generic utilities layer:

- **Build and bootstrap** own deterministic ESM and classic-script artifacts,
  validated startup configuration, singleton startup, version compatibility,
  CSP-safe loading, and bounded diagnostics.
- **Island ownership** owns root metadata parsing, seed-or-instance identity,
  connection records, document-order discovery, nested boundaries, dynamic
  insertion, disconnection, and proposed first-action nonces.
- **Directive handling** owns the closed `live:` grammar, parser, ownership
  resolution, delegated DOM listeners, event modifiers, and checker/runtime
  conformance data.
- **Local interaction** owns typed island-scoped signals, presentation
  directives, local accessibility state, optimistic projections, registered
  effects, public runtime calls, and the optional Stimulus lifecycle bridge.
- **Interaction scheduling** owns one bounded scheduler per island, model timing
  and coalescing, semantic request construction, transport cancellation and
  retry, dirty state, and truthful feedback transitions.
- **Response application** owns protocol validation, correlation/revision
  eligibility, redirect precedence, morph/no-render preflight, commit-after-
  morph, validation and focus reconciliation, events, effects, URL intent,
  recovery, and final feedback settlement.
- **DOM reconciliation** owns matching-root validation, keyed identity,
  Idiomorph adaptation, nested-island opacity, form/focus/selection continuity,
  preserve/ignore/replace/persist/teleport controls, signal/controller
  continuity, transitions, and controlled replacement after failure.
- **Document enhancement** owns native prefetch declarations, same-route
  `replaceState` reflection, cross-document View Transition enhancement,
  history/focus/scroll behavior, dirty-work guards, bfcache restoration, and
  document-scoped cleanup. It never fetches and installs a partial document.

There is one document runtime and one scheduler per connected island. No global
application store, global component registry, client component object graph, or
browser authority is introduced. Document-scoped registries are created by the
runtime instance and disposed with it.

## Dependency boundaries

Idiomorph 0.7.4 is pinned and included in production artifacts behind the
framework-owned morph adapter. Its matching behavior is never exposed as the
public Live identity contract. Conformance fixtures must remain valid if the
implementation is replaced.

Stimulus 3.2 is optional. The core runtime does not import or bundle it. A bridge
integrates with an application-supplied Stimulus `Application` through public
connect/disconnect and morph lifecycle contracts. Standard Stimulus attributes
remain ordinary template markup.

Build, browser-test, and packaging tools are exact lockfile dependencies. They
do not become runtime dependencies or application requirements. The final ESM
and classic artifacts include the same core behavior and protocol version,
exclude source maps from the production asset set by default, and are checked
for reproducibility and transfer-size budgets.

## Data flow

### Startup and discovery

1. A canonical document exposes initial HTML and inert bounded runtime metadata.
2. The production artifact validates the document configuration and establishes
   one runtime instance. A duplicate load reuses or rejects that instance
   without duplicating listeners or observers.
3. Discovery walks valid island roots in deterministic document order. It
   validates component, slot, protocol, snapshot form, endpoint, and root
   ownership before creating an island record.
4. Instanced islands retain their encoded snapshot and accepted revision.
   Seed-backed islands remain server-connectionless; the browser creates an
   untrusted Web-Crypto random 128-bit-or-stronger proposed nonce only when their
   first server action is built, retains it for that intent's permitted retries,
   and discards it when promotion resolves. No predictable fallback exists.

### Local interaction

1. Delegated event resolution stops at the nearest owning island and excludes
   nested child-island content from the parent.
2. A local directive updates only its typed signal and declared presentation
   targets. It performs no transport and cannot write snapshot, revision,
   authorization, or accepted-outcome state.
3. Accessibility attributes, visibility, focusability, inertness, and reduced-
   motion behavior change as one coherent local presentation update.

### Server interaction

1. A model or action directive produces a bounded scheduler intent associated
   with one island, source directive, semantic idempotency identity, and current
   browser proposal state.
2. Model timers coalesce permitted unsent values. Submit flushes the newest
   eligible proposals into exactly one ordered action intent.
3. Transport sends the existing versioned request envelope with credentials and
   host-defined endpoint policy. Browser-generated correlation data is never
   mistaken for authority.
4. The scheduler validates the complete response before application. Redirect
   and protocol-v2 navigated URL intent are terminal and use normal navigation.
5. A non-redirect response preflights its island root and state-machine plan. A
   successful morph or validated no-render phase occurs before the browser
   commits the successor snapshot and revision.
6. The runtime then reconciles model and validation state, restores focus,
   schedules signed child-parameter work, applies same-route URL reflection,
   dispatches registered events and effects, and settles feedback in the
   protocol-defined order. Child work enters each child's scheduler and is not
   atomic with the accepted parent morph.

### Failure and recovery

Malformed configuration, metadata, directives, protocol values, HTML roots, or
extension output fail closed before protected state changes. Diagnostics use
bounded closed codes and redact snapshots, signatures, field values, tokens,
cookies, HTML bodies, and arbitrary URLs.

Transport interruption retains the last accepted DOM and snapshot. Automatic
retry is bounded and reuses the same idempotency identity only when the request
contract permits it. Canceling browser work stops future application but never
claims server rollback.

If response eligibility or morph safety cannot be proved, the runtime does not
install the successor snapshot over the prior DOM. It requests a fresh render
without replaying the original action. Repeated recovery failure disconnects
only the affected island and leaves its SSR or last accepted content exposed.

Extension, effect, and controller failures remain scoped where possible and
cannot bypass protocol validation, ordering, or morph preflight. A failed
effect does not roll back or disguise an already accepted server outcome.

## DOM continuity contract

The returned island root must match the current island's component, slot, and
successor context before mutation. Nested island roots are opaque boundaries.
Stable keys move existing logical identity; ambiguous or duplicate keys fail
preflight rather than transferring state speculatively.

Morph capture records focus, focus-visible state, compatible selection/range,
composition state, dirty control proposals, scoped scroll positions, surviving
signal roots, controller roots, and declared persistence/teleport identity.
Reconciliation applies authoritative attributes while retaining newer permitted
browser edits. Commit restores captured state only to the matching surviving
identity. Removed identity disposes resources once and uses the declared focus
fallback.

Idiomorph performs the internal tree reconciliation only after Live preflight.
Live owns prohibited-node handling, scripts, keys, nested islands, preservation,
lifecycle hooks, commit ordering, and recovery. A morph never executes returned
scripts incidentally.

## Navigation model

All route transitions remain anchors, forms, redirects, refreshes, or browser
history operations targeting complete canonical documents. Live may emit native
prefetch or Speculation Rules declarations for eligible safe requests; it never
stores a JavaScript-fetched document body for partial installation.

URL reflection is limited to same-route query `history.replaceState` and does
not install a `popstate` Live action path. Cross-document View Transitions are
feature-detected visual enhancement. Unsupported capability, reduced motion,
capture failure, or incompatible navigation always falls back to ordinary
navigation with identical semantic results.

Page hide/freeze retires or suspends document resources. Pages restored through
bfcache validate runtime and island compatibility, discard stale in-flight
application, and reconnect without duplicating listeners, observers,
controllers, or subscriptions.

## Verification strategy

Pure deterministic modules use Vitest: configuration, grammar parsing, signal
state, scheduler transitions, coalescing, feedback, protocol eligibility,
ordering, retry, lifecycle, and diagnostic redaction. Shared versioned fixtures
are consumed by Rust and TypeScript for directive grammar, metadata, protocol,
response plans, morph cases, and compatibility.

Real DOM behavior uses Playwright against served production artifacts. Browser
tests cover discovery, nested ownership, delegated events, local directives,
multiple islands, seed first-action nonce behavior, transport ordering, forms,
dirty edits, focus, selection, IME composition, keyed reorder, controllers,
signals, preservation controls, teleports, transitions, redirects, URL
reflection, offline/retry, cancellation, page lifecycle, bfcache, CSP, duplicate
runtime loading, and failure recovery.

The normal gate pins Playwright Chromium, Firefox, and WebKit. A separate
provider-neutral compatibility runner records results from the actual minimum
Chrome/Edge 111, Firefox 128, and Safari 16.4 floors and current stable releases.
Playwright WebKit is useful conformance evidence but is never labeled Safari.
Missing validated floor evidence blocks release qualification rather than being
silently treated as a passing compatibility claim.

During implementation, `agent-browser` supplies fast exploratory and dogfood
verification through accessibility-tree snapshots, semantic element refs,
network controls, screenshots, and accessibility audits. Its sessions are
derived per worktree, element refs are refreshed after every DOM change, and
arbitrary waits are not accepted as correctness evidence. DevTools MCP may be
used for network, lifecycle, performance, retained-memory, observer, bfcache,
and accessibility diagnosis. These agent-operated tools complement rather than
replace checked-in Playwright cases, shared fixtures, and reproducible benchmark
records, and neither is a shipped runtime dependency.

Accessibility assertions cover semantic states, keyboard operation, focus
recovery, live-region behavior, reduced motion, and feedback truth. Automated
checks supplement explicit keyboard and assistive-technology review fixtures.

The `D100`, `M1K`, and `M5K` workloads enforce bootstrap, idle, retained-memory,
morph-latency, and core-runtime transfer budgets. Release-grade B1 claims require
the exact pinned environment and at least 30 post-warmup samples; local results
remain honestly labelled exploratory. Leak tests repeat connect, morph, remove,
document freeze, and restore cycles under deterministic observers and clocks.

## Scope boundary

Iteration 003 does not modify or integrate the active Suprnova or Magnetar
worktrees. It does not implement uploads, server-pushed streams, RenderCache,
deployment-tier providers, the official component library, CLI scaffolding, the
dogfood application, or final framework asset/router registration. It adds no
SPA navigation, client renderer, arbitrary JavaScript expression language,
server-returned executable code, persistent default client store, or synthesized
no-JavaScript Live action path.

The next integration decision remains evidence-driven: development stays in the
dedicated workspace until the separation materially blocks a coherent change.
