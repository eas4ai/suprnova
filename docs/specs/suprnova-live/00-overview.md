# Suprnova Live -- System Overview

Status: Normative design specification
Last revised: 2026-09-01

## Purpose

Suprnova Live shall provide Suprnova developers with an internal,
server-driven way to build rich, reactive web interfaces without adopting a
client component framework. Real Suprnova routes must return canonical,
server-rendered HTML documents; optional RenderCache shall avoid repeated work
where a representation remains valid; and independently owned Live islands
shall support targeted interaction through typed Rust component state, server
actions, browser-local signals, and identity-preserving DOM morphs.

The system shall make Livewire-like application development coherent with
Rust, Suprnova's existing application services, and normal HTTP semantics
without reducing the experience to page-wide rerenders or basic form
interactions. It is complete when the specified component model, browser
runtime, rendering contracts, cache-coherence model, developer tooling, and
official component library work together as one adoption-grade Suprnova
frontend mode, including validation, accessibility, transitions, animation,
and recovery from failed or concurrent interactions.

## Design principles

1. **Real routes and HTML are the ground truth.** A route returns a complete,
   meaningful canonical document, never a JavaScript bootstrap shell or a
   client-routing protocol.
2. **Update the owning island, not the page.** A Live action rerenders and
   morphs only its independently identified island; unrelated document work
   must not be repeated.
3. **The server remains authoritative.** Rust component state, actions,
   validation, authorization, and domain effects are decided on the server;
   browser input is an untrusted proposal.
4. **Keep local interaction local.** Disclosure, toggles, focus behavior,
   animation state, and similar non-authoritative behavior should use local
   signals or browser controllers without unnecessary server requests.
5. **Cache validity is a correctness contract.** RenderCache reuse must be
   justified by explicit variance, dependency generations, and a coherence
   policy rather than TTLs or best-effort deletion alone.
6. **Private state must not poison shared output.** Public cached content and
   request-specific or identity-bound islands must remain distinguishable and
   safely composable through server stitching.
7. **Preserve browser continuity.** Morphing must respect keyed DOM identity,
   focus, form state, local signals, controller lifecycles, transitions, and
   explicit preservation boundaries.
8. **Live is a progressive enhancement boundary, not a fallback generator.**
   Initial content is exposed without JavaScript, while Live directives and
   actions require the Live browser runtime; the framework does not synthesize
   alternate no-JavaScript action paths.
9. **Live owns a coherent frontend mode.** Its runtime and protocol must not be
   coupled to Inertia, Turbo, an SPA router, or a client virtual DOM. Suprnova
   may offer those approaches separately.
10. **Own contracts; isolate replaceable machinery.** Suprnova defines the
    component, view, wire, morph, and cache contracts behind internal
    boundaries so an implementation dependency does not become the public
    architecture.
11. **Server-driven must not mean interaction-poor.** Accessibility, responsive
    feedback, optimistic local behavior, transitions, animation, and custom
    browser controllers are first-class requirements rather than escape
    hatches.
12. **Framework features ship as a system.** Sequencing may reduce development
    risk, but it must not silently redefine agreed functionality as an MVP or
    leave developers with a narrow subset that cannot support real
    applications.
13. **Tier 0 is complete, not degraded.** Live works without RenderCache, and
    Embedded RenderCache works without an external daemon. Database and
    networked key/value tiers change topology and performance rather than
    application features, trust guarantees, or cache correctness.

## System architecture

Suprnova Live is one internal Suprnova subsystem with a dependency-inverted
engine crate, framework integration, generated declarations, a browser runtime,
and an official component catalog. Applications consume only the
`suprnova::live`, `suprnova::view`, and CLI surfaces. They do not depend on the
internal engine crate directly.

```text
application routes and Live components
                 |
                 v
       suprnova framework facade
       routing / middleware / auth
       sessions / ORM / events / cache
                 |
                 v
  internal crates/suprnova-live engine
  component loop / snapshots / protocol
  rendering contracts / RenderCache ports
       |                         |
       v                         v
 Askama views and checker   provider implementations
       |                    memory / file / database /
       v                    networked key-value cache
 canonical HTML
       |
       v
 Suprnova browser runtime <----> versioned Live endpoint
 local signals / scheduler / Idiomorph adapter + optional feature adapters
```

### Workspace and ownership boundaries

- `crates/suprnova-live/` is the internal engine crate. It owns component,
  snapshot, protocol, rendering, scheduling metadata, RenderCache contracts,
  provider conformance fixtures, the browser-runtime source, and component
  assets. It must not depend on the public `suprnova` framework crate.
- During dedicated development, this repository owns the implementation,
  `docs/specs/suprnova-live/` as the normative Live specification root, and
  `scripts/check-specs.mjs` as their structural drift gate. Development starts
  here rather than modifying the active Suprnova checkout. When separation
  materially blocks integration, testing, or coherent changes, the product
  tree, normative specifications, and checker move together into
  `suprnova/crates/suprnova-live/`; neither repository may remain a parallel
  maintained authority afterward.
- Iteration 005 authorizes that atomic move. Authority transfers when the
  committed standalone history and the complete checked product/specification
  tree land beneath `suprnova/crates/suprnova-live/`. The standalone repository
  then remains historical evidence only; uncommitted local files, reference
  catalogs, ignored dependencies, and optional handoff archives do not silently
  join the product import.
- Before that atomic move, the standalone engine exposes internal Live host
  adapter contracts for normalized request facts, verified request context,
  application services, and typed response intent. Conformance and test
  adapters exercise those contracts without claiming to be Suprnova
  integration. The standalone workspace has no source or path dependency on
  the active Suprnova checkout.
- `framework/src/live/` is the Suprnova integration and public facade. It adapts
  the router, request context, middleware, sessions, authorization, SeaORM,
  generic cache, events, broadcasting, telemetry, and configuration to the
  internal engine and re-exports the application-facing API.
- `suprnova-macros/` owns Live procedural macros and generated metadata. Macro
  output names only public `::suprnova::live` paths so applications never bind
  to internal crate layout.
- `suprnova-cli/` owns Live scaffolding, inspection, the cross-language check
  command, asset installation, and generated-project drift fixtures.
- `app/` is the end-to-end dogfood application. It must exercise SSR-only pages,
  Live pages with and without RenderCache, all three deployment tiers where
  practical, and the official component families.
- The official component library ships with Suprnova Live but remains separable
  from the CSS-agnostic runtime. Its templates, Tailwind CSS 4 source, semantic
  theme tokens, catalog fixtures, and accessibility tests are versioned with the
  Live contract.
- The large reference catalog remains in the development repository. Its pinned
  provenance informs implementation, but references are evidence rather than a
  normative contract and do not justify splitting specifications from product
  code.

### Server execution paths

An ordinary document request enters the normal Suprnova middleware and route
pipeline. The handler renders through `suprnova::view`; Askama produces the
canonical HTML while the request-scoped dependency collector records all
observable inputs. With RenderCache disabled, the response is sent normally.
With RenderCache enabled, a proven Complete hit bypasses the handler and
renderer, while a Composite hit performs typed server stitching before final
headers and validators are produced.

A Live request enters an explicit versioned framework endpoint through the same
session, CSRF, origin, tenant, authorization, rate, and observability boundaries
as other state-changing requests. The engine validates the envelope and signed
snapshot, claims the expected island revision through the configured instance
ledger, hydrates only registered state, invokes declared operations, renders the
owning island, and returns one typed outcome. Browser state advances only after
the runtime validates the response and completes the required morph or
no-render phase.

Push transports carry typed events, invalidations, or presentation-only local
data into the existing island scheduler. They do not introduce a second HTML
patch protocol, bypass revision authority, or make a persistent connection a
prerequisite for Live. They never automatically invoke a mutating Live action;
ordinary server/application paths own domain mutation and may publish an
invalidation afterward. A reconnect becomes current only after trusted replay
proves continuity or an authoritative refresh establishes a new baseline.

Uploads separate an opaque non-authority handle from a short-lived secret
transfer grant. The standalone reverse-proxy/file provider streams untrusted
bytes into quarantine without a daemon, while provider-neutral direct-storage
adapters preserve the same revisioned lifecycle, verification, finalization,
and cleanup contracts. File bytes and transfer grants never enter component
snapshots or normal Live action envelopes.

### Technology choices

| Area | Choice and rationale |
|---|---|
| Server foundation | Rust 2024 on Suprnova's pinned MSRV, Tokio, hyper, and SeaORM. Live extends the framework's existing request and application-service boundaries instead of creating a parallel stack. |
| Template substrate | Askama 0.16 is the normative checked external-template substrate behind `suprnova::view`. It provides compile-time Rust integration and a concrete grammar for the Live checker without becoming the public handler API. |
| Browser runtime | Strict TypeScript compiled to versioned ESM and classic-script artifacts targeting ES2020. A universal core plus manifest-selected optional Stimulus, upload, and asynchronous feature pairs avoid charging every page for uncommon capabilities. Suprnova ships all production artifacts, so applications need neither a client framework nor a JavaScript build step merely to use Live. |
| Local controllers | Stimulus 3.2 is the supported opt-in controller substrate. Neither Stimulus nor Suprnova's bridge implementation is bundled into or required by the Live core runtime. A separately shipped adapter pair owns controller continuity through the core's closed ordered lifecycle driver while preserving the application-supplied `Application` contract. |
| DOM reconciliation | Idiomorph 0.7.4 is pinned and vendored behind Suprnova's morph adapter. Suprnova owns preflight, keys, preservation, lifecycle, commit ordering, and recovery rather than exposing Idiomorph as the contract. |
| Wire representation | Versioned JSON control protocols keep requests inspectable. Protocol v1 is the trusted action spine; v2 adds component lifecycle operations and typed child/URL outcomes without changing snapshot schema v1. Signed snapshot bodies and semantic idempotency digests use purpose-specific versioned RFC 8785-compatible canonical JSON profiles; ordinary transport JSON need not be canonical when it is not signed. |
| Upload and event protocols | Upload control/data and asynchronous event envelopes are independently versioned bounded protocols rather than a generic Live v3 transport. SSE and WebSocket share event semantics; reverse-proxy/file and direct-storage providers share upload authority and lifecycle semantics. |
| Snapshot integrity | Purpose-separated keys derived with HKDF-SHA-256 from Suprnova's configured key ring sign canonical snapshot bytes with HMAC-SHA-256. Explicit key identifiers and overlap windows support rotation; signatures provide integrity, never secrecy or authorization. |
| Component styling | The official library targets Tailwind CSS 4 and semantic CSS theme tokens. The runtime itself owns no required stylesheet and remains usable with application-defined CSS. |
| Provider model | `RenderStore`, `LiveInstanceLedger`, `RebuildCoordinator`, and `GenerationLedger` are independent contracts. Embedded, database-coordinated, and externally accelerated profiles select adapters without changing application code or semantics. |

### State, cache, and deployment topology

Component objects are reconstructed per request from verified snapshots. Public
cache-safe islands begin with reusable seed snapshots and create scoped ledger
state only on their first server action. Instanced snapshots carry state but do
not replace ledger revision authority, current authorization, or authoritative
domain reads.

RenderCache is an optional layer above normal rendering, not a prerequisite for
Live and not an alias for `suprnova::Cache`. Generation truth remains in the
application database at every tier. Memory, files, database blobs, Redis,
Memcached, or similar key/value stores may retain bytes and coordination state,
but they cannot become generation authority. Tier 0 supplies the provider
conformance reference and every correctness guarantee without an external
daemon.

## Cross-cutting requirements

### API and compatibility

- Application code imports Live through `suprnova`, never through the internal
  crate or a browser dependency's private API.
- Clippy findings are reviewed and resolved, but the gate does not blanket-promote
  warnings to errors with `-D warnings`. An intentional suppression is narrow,
  uses `#[allow(..., reason = "...")]`, and records why the lint does not express
  a defect at that site.
- Component, view metadata, directive grammar, protocol, snapshot, cache-entry,
  and runtime versions evolve independently and declare their compatibility
  windows explicitly.
- A rolling deployment either supports the observed version pair or returns one
  bounded fresh-render or document-refresh instruction. It never guesses.
- Live and Inertia remain separate frontend modes. Shared Suprnova domain
  services do not imply a mixed rendering, navigation, or browser protocol.

### Security and privacy

- Every browser value is untrusted. Snapshot verification precedes hydration or
  expensive work, and current middleware, authorization, tenant, and domain
  checks follow verification before protected effects.
- Snapshot, protocol, upload, template, cache, and effect parsers have explicit
  depth, count, byte, time, and allocation limits and receive fuzz/negative
  coverage.
- Secrets, transfer grants, subscription credentials, and transient model values
  never enter HTML, snapshots, cache bodies, URLs/history, logs, metrics,
  diagnostics, browser effects, or exception text.
- CSP-safe external assets, registered effects, escaped templates, safe URL
  handling, purpose-separated keys, and constant-time signature verification
  are release requirements.
- Cache classification fails toward private or uncacheable. No public entry may
  contain principal-bound, tenant-private, or instanced state.

### Correctness and failure behavior

- Signed and cached representations use deterministic canonical encodings;
  nondeterministic application values must be declared dependencies or excluded.
- An action body is safe to invoke more than once before commit. Live guarantees
  at most one committed accepted outcome per base revision, not exactly-once
  method invocation or external side effects.
- Redirect, morph, snapshot commit, validation reconciliation, events, effects,
  and feedback follow the one response state machine defined by the protocol.
- Cancellation, provider eviction, partial failure, deployment mismatch, and
  stale browser state fail through typed recovery. They cannot publish partial
  output, replay an accepted action, or manufacture authority.
- Clocks, randomness, providers, schedulers, and network ordering are injectable
  in tests; correctness suites do not depend on sleeps.

### Accessibility and browser support

- Initial HTML and official components meet WCAG 2.2 AA semantics, keyboard,
  focus, labeling, error, contrast, target-size, and reduced-motion contracts.
  Automated checks supplement manual assistive-technology review of critical
  flows.
- The supported baseline is the Tailwind CSS 4 browser floor: Safari 16.4,
  Chrome and Edge 111, and Firefox 128 or newer. Optional capabilities such as
  View Transitions and Speculation Rules are feature-detected and cannot change
  semantic outcomes.
- Browser behavior is tested at the oldest supported floor and current stable
  releases. Compatibility changes require a dated overview revision and release
  note.

### Scalability, topology, and operations

- A single-process SQLite application can use Live and complete Tier 0
  RenderCache semantics with no daemon. A small multi-node application can use
  its shared database alone. External key/value infrastructure is a performance
  choice.
- All provider adapters pass Tier 0 behavioral conformance. Distributed adapters
  additionally prove CAS, fencing, eviction, lease-expiry, partition, and
  bounded-staleness behavior.
- Metrics and traces use bounded labels and correlate document, island, action,
  render, morph, cache, generation, and rebuild work without payload leakage.
- Backpressure bounds action queues, uploads, rebuild fan-in, push delivery,
  cache assembly, and diagnostic retention.

### Architecture performance budget v1

Budget v1 uses two reproducible environments. `S1` is a release build on a
Linux x86-64 runner with eight dedicated vCPUs, 16 GiB RAM, the performance CPU
governor, warm filesystem cache, and loopback providers; the exact CPU, kernel,
database, and provider versions are recorded with every baseline. `B1` is the
same runner using a pinned Playwright Chromium, a 1280x720 viewport, four-times
CPU throttling, no extensions, and a warm HTTP cache. Each release result uses
at least 30 measured samples after warmup and reports p50 and p95.

Canonical workloads are: `D100`, a 64 KiB document containing 100 connected
islands; `A8/16`, an 8 KiB snapshot and no-domain-I/O action returning 16 KiB of
island HTML; `M1K`, a keyed 1,000-element island with maximum depth 12 and ten
percent changed nodes; `M5K`, a keyed 5,000-element island with maximum depth 24
and ten percent changed nodes; `C64`, a 64 KiB Complete representation with 12
dependencies; `C64+4`, the same public base with four 4 KiB stitch slots;
`U4/16`, four concurrent 16 MiB uploads in 256 KiB chunks through the loopback
reference provider; `E100/1K`, 100 subscribed islands receiving 1,000 ordered 1
KiB presentation events over ten seconds with ten-percent refresh
invalidations; and `R100`, simultaneous continuity loss plus jittered recovery
for those 100 subscriptions.

| Budget | v1 target |
|---|---|
| Core runtime transfer size | Every deterministic build measures and reports exact Brotli bytes for both production core variants, including the pinned morph implementation and excluding optional Stimulus, diagnostics, source maps, and component CSS. Core transfer size has no absolute release-blocking ceiling until the universal core is functionally complete and an evidence-based baseline plus explicit maintenance headroom is approved. |
| Optional Stimulus adapter | Every deterministic build reports exact Brotli bytes for each Stimulus bridge ESM/classic production artifact; there is no absolute ceiling. It contains Suprnova's bridge and continuity implementation, imports or bundles no Stimulus package, and loads only when an application supplies a Stimulus `Application`. |
| Optional upload artifact | Every deterministic build reports exact Brotli bytes for each upload ESM/classic production artifact, including its required bounded-resource implementation; there is no absolute ceiling. It loads only for a document whose trusted checked metadata requires the upload role. |
| Optional asynchronous artifact | Every deterministic build reports exact Brotli bytes for both asynchronous-update production variants. There is no absolute ceiling and no drift rule; growth is read from the reported bytes and reviewed by a person. The artifact loads only for a document whose trusted checked metadata requires the async role. |
| Optional driver claims | One document retains at most 256 active island ports in the optional lifecycle driver. Island 257 fails optional-capability admission with one bounded `resource_exhausted` diagnostic while ordinary Live and earlier admitted islands remain operational; retirement releases capacity. |
| Runtime bootstrap | `D100` connects in at most 50 ms p95 on `B1`; 30 idle seconds consume at most 5 ms total main-thread time, use at most one core mutation observer per document, and perform no polling or network request. |
| Runtime memory | At most 12 KiB incremental retained runtime memory per connected island in `D100`, excluding DOM nodes and the raw HTML/snapshot byte strings owned by the document. |
| Morph latency | `M1K` completes in at most 32 ms p95 and `M5K` in at most 100 ms p95 on `B1`, including Live preflight, lifecycle hooks, reconciliation, and browser-state commit. |
| Protocol overhead | Each request and response adds at most 1 KiB fixed control-envelope bytes; a signed snapshot adds at most 768 bytes beyond application state and lifecycle memo for identity, version, timing, and integrity fields. |
| Snapshot processing | Verify, hydrate, dehydrate, canonicalize, and sign `A8/16` state in at most 500 microseconds p95 on `S1`, excluding component hooks and rendering. |
| Complete L0 hit | Validate and produce `C64` in at most 250 microseconds p95 on `S1`, with no database/provider round trip, no handler/template execution, at most four heap allocations, and no full-body copy after shared-byte retrieval. `crates/suprnova-live/benches/render_cache_budget.rs` measures the allocation limit with its benchmark-only counting global allocator in an isolated serialized process. |
| Warm file L1 hit | Validate and produce `C64` from the warm Embedded file store in at most 2 ms p95 on `S1`, excluding socket transfer. |
| Action framework overhead | `A8/16` adds at most 2 ms p95 on `S1` outside application action, domain I/O, and Askama render time. Tier 0 adds no coordination network trip; Tier 1 adds at most one transaction-coupled ledger CAS/write; Tier 2 adds at most one key/value CAS operation. |
| Composite assembly | Assemble `C64+4` in at most 2 ms p95 on `S1`, excluding slot rendering and provider I/O, with at most two full-response-sized byte copies. |
| Publication generation reread | One publication performs one batched fresh authority query and spends at most 3 ms p95 on the `S1` loopback database for 12 dependency keys. |
| Upload resource envelope | `U4/16` retains at most two configured chunk buffers per active transfer plus 256 KiB browser-manager overhead and two configured chunk buffers per active server transfer plus 512 KiB server-manager overhead. Progress application is at most 16 ms p95 on `B1`; control-plane framework overhead is at most 2 ms p95 on `S1`, excluding body I/O, provider work, scanning, and application validation. |
| Asynchronous event envelope | `E100/1K` retains at most 8 KiB framework memory per active subscription excluding native transport, DOM, and the currently dispatched payload. Queued unapplied browser events are capped at 64 items and 256 KiB per document; typed presentation dispatch is at most 8 ms p95 on `B1`; invalidations retain at most one queued plus one in-flight refresh per island. |
| Reconnect storm | `R100` permits at most eight concurrent reconnect handshakes per origin, creates no synchronized polling burst, and returns within the 12 KiB retained-runtime-per-island cap after currentness is reestablished. |

These budgets are measured by the on-demand benchmark tools and reported as
numbers. No budget is a condition of the ordinary gate, which verifies
correctness and security only. Whether a reported number is acceptable for a
release is a review judgment recorded in the decisions log, not an automated
cliff, and a baseline is never derived from the candidate it describes.
Correctness and security suites run beside the fast path so an unsafe shortcut
cannot pose as a budget win.

## Spec map

| Spec | Owns |
|---|---|
| `01-views-and-documents.md` | Canonical route documents, the Suprnova view contract, render context, and initial island mounting |
| `02-component-lifecycle-and-composition.md` | Component registration, mount/hydrate/render hooks, nested ownership, and parent-child parameters |
| `03-component-state-and-binding.md` | State categories, typed model proposals, transient fields, computed state, and URL reflection |
| `04-actions-and-validation.md` | Registered Rust actions, validation, authorization boundaries, transactions, and semantic outcomes |
| `05-snapshots-and-hydration.md` | Seed and instanced snapshot schemas, promotion, canonical state, hydration, revisions, and recovery |
| `06-wire-protocol-and-transport.md` | Versioned endpoint envelopes, response ordering, idempotency, errors, and rolling compatibility |
| `07-security-and-trust-boundaries.md` | Threat model, signing, request authenticity, tenant isolation, browser security, and abuse resistance |
| `08-file-uploads.md` | Temporary upload identity, transfer, quarantine, finalization, cleanup, morphing, and accessibility |
| `09-runtime-bootstrap-and-directives.md` | Runtime delivery, island discovery, directive grammar, delegated events, lifecycle, and configuration |
| `10-local-reactivity-and-javascript-interop.md` | Local signals, presentation directives, Stimulus integration, registered effects, and optimistic projection |
| `11-interaction-scheduling-and-feedback.md` | Per-island queues, coalescing, feedback, stale suppression, offline behavior, and cancellation |
| `12-dom-morphing-and-identity.md` | Bounded Idiomorph adaptation, keys, focus/forms, preservation controls, lifecycle continuity, and recovery |
| `13-document-navigation-and-transitions.md` | Real-route navigation, native prefetch, document transitions, URL semantics, history, and bfcache |
| `14-events-and-asynchronous-updates.md` | Component/browser events, WebSocket/SSE augmentation, typed streams, reconnect, and backpressure |
| `15-render-representations-and-storage.md` | Render policy, Complete/Composite formats, layered byte storage, validators, and store failures |
| `16-cache-variance-privacy-and-stitching.md` | Variance, privacy classification, private keys, typed server stitching, and composition safety |
| `17-dependency-tracking-and-generations.md` | Handler-wide dependency collection, ORM/config/custom keys, transactional generations, and publication rereads |
| `18-cache-coherence-and-rebuilding.md` | Validation, deployment tiers, generation authority, leases, invalidation, stale policy, and fenced rebuilding |
| `19-developer-tooling-and-testing.md` | Macros, view checking, CLI, harnesses, conformance, observability, benchmarks, and release gates |
| `20-component-library-foundations.md` | Component anatomy, variants, state, accessibility, Tailwind 4 tokens, catalog, and release discipline |
| `21-form-and-input-components.md` | Fields, controls, choices, secret inputs, validation display, uploads, and form composition |
| `22-navigation-components.md` | Links, breadcrumbs, tabs, pagination, menus, active route state, and responsive navigation |
| `23-overlay-and-disclosure-components.md` | Dialogs, drawers, popovers, tooltips, menus, accordions, teleportation, focus, and layering |
| `24-feedback-and-status-components.md` | Alerts, toasts, progress, loading, empty/error states, feedback truth, and announcements |
| `25-data-display-and-layout-components.md` | Tables, lists, cards, metadata, layout primitives, responsive density, and visualization integration |

Companion specifications are `glossary.md` for normative vocabulary, `ux.md`
for journeys and cross-domain interaction, and `conventions.md` for
implementation and verification rules.

## Supported and excluded scope

### Supported

- Canonical server-rendered documents served by real Suprnova routes using
  Askama as the normative checked external-template substrate behind
  Suprnova's view contract.
- Stateful Live component semantics over stateless requests: mounting, typed
  component state, explicitly exposed model binding, registered server
  actions, validation, errors, events, effects, and lifecycle handling.
- Versioned public seed and instanced signed snapshots, first-action promotion,
  atomic authority creation for identity-bound initial mounts, an expiring
  tier-provided instance ledger, one committed outcome per base revision,
  idempotency, expiration, and recovery behavior.
- An independently shipped Suprnova browser runtime for Live directives,
  action transport, model synchronization, local signals, effects, scheduling,
  and bounded DOM morphing.
- Browser-local behavior and custom controller integration for interactions
  that do not require server authority or computation.
- Identity-preserving island morphs, including explicit keys, preservation and
  replacement controls, focus and form handling, controller continuity, and
  transition and animation integration.
- Optional RenderCache with Complete and Composite representations, handler-wide
  dependency collection, transactional logically append-only database
  generations, fresh publication rereads, cache variance, server stitching,
  private representations, and explicit coherence policies across Embedded,
  Database-coordinated, and Externally accelerated tiers.
- Normal document navigation, with optional prefetching and visual transitions
  that preserve real route and browser semantics.
- Integration with Suprnova's application facilities, including middleware,
  authentication, authorization, sessions, validation, persistence, events,
  queues, WebSockets, broadcasting, and ordinary HTTP handlers, without
  duplicating their domain responsibilities inside Live.
- Developer-facing compile-time or build-time contract checking, diagnostics,
  test support, observability hooks, and security-sensitive defaults.
- An official accessible Suprnova Live component library styled with Tailwind
  CSS 4 and driven by theme tokens, while the Live runtime itself remains
  independent of any required CSS framework.

### Excluded

- A third-party or framework-independent crate. Suprnova Live is developed here
  as an internal Suprnova subsystem and shall ultimately live within the
  Suprnova project boundary.
- An SPA architecture, client-side router, JSON page protocol, virtual DOM, or
  general-purpose client component framework.
- An Inertia adapter or a mixed Live/Inertia rendering protocol. Inertia remains
  a separate Suprnova frontend mode.
- Turbo-style partial document navigation or any navigation mechanism that
  replaces the authority of real routes and canonical documents.
- Synthesized no-JavaScript handlers, automatic action parity, or alternate
  fallback transports for Live directives and actions. Applications may write
  ordinary Suprnova routes, forms, and links explicitly when equivalent
  no-JavaScript interaction is required.
- Browser authority over domain, authorization, session, or security state;
  snapshot signatures are integrity controls, not authorization proofs or
  secrecy mechanisms.
- Persistent server-resident component objects as the default component-state
  model.
- A mandatory Redis, Memcached, cache daemon, distributed coordinator, or
  RenderCache deployment merely to use Suprnova Live.
- Whole-document rerendering, wholesale island replacement as the normal update
  mechanism, or loss of unrelated island state after a Live action.
- A mandatory component CSS framework or a requirement that applications use
  the official Tailwind component library.
- A visual theme-authoring studio. Theme tokens and component compatibility may
  support that separately scoped feature, but the studio is not part of this
  system specification.

## Revision policy

- Normative specifications are revised in place, retain their stable filenames,
  update `Last revised`, and add a concise newest-first entry to the affected
  Decisions and revisions section.
- A cross-domain contract changes atomically across the owning spec, every
  dependent cross-reference, the glossary when terminology changes, overview
  architecture or budgets when applicable, fixtures, and implementation.
- Acceptance criteria may be strengthened or clarified in place. Weakening,
  removing, or deferring an agreed capability requires an explicit developer
  decision recorded with the rejected behavior and reason.
- Protocol, snapshot, directive, cache-entry, view-checker, and runtime versions
  are independent. A breaking revision names its compatibility window,
  migration/recovery behavior, conformance fixtures, and deployment impact.
- Pinned upstream machinery is updated only through an explicit compatibility,
  security, license, size, and conformance review. Upstream behavior never
  silently overrides a Suprnova-owned contract.
- Once Stage 6 establishes `iterations/001.md`, new ideas and scope changes enter
  through `/next-iteration`; implementation may not use an attractive adjacent
  feature to bypass the active scope contract.
- A disagreement between code and spec is drift, not an informal exception.
  Work cannot be called done until code, tests, generated artifacts, references,
  and normative text agree.

## System completion criteria

Suprnova Live is complete when all of the following are true:

- Implementation begins in the dedicated `suprnova-live` workspace with its
  normative specifications and structural checker colocated. If the separation
  later becomes a blocker, integration moves the product tree, specifications,
  and checker into `crates/suprnova-live/` together and leaves no parallel
  maintained authority.
- Every acceptance criterion in specs 01 through 25 is implemented and covered
  by the appropriate Rust, macro UI, protocol fixture, browser, accessibility,
  security, provider-conformance, and benchmark evidence.
- The internal engine crate is integrated through `suprnova::live` and
  `suprnova::view`, documented, re-exported, scaffolded, and usable without an
  application importing internal crates.
- Real routes render canonical Askama-backed documents; Live actions update only
  the owning island; initial content remains exposed when the runtime is absent.
- Generated Rust metadata, Askama templates, checker grammar, wire fixtures, and
  browser directives reject incompatible declarations during normal check or
  deterministic runtime validation.
- Seed promotion, instanced snapshots, purpose-separated signing, instance
  revisions, response ordering, commit-after-morph, refresh recovery, transient
  models, and signed child parameters pass adversarial and concurrency tests.
- Local signals, Stimulus controllers, effects, morphing, uploads, navigation,
  transitions, events, push augmentation, offline behavior, and accessibility
  work together without creating client routing or browser authority.
- RenderCache can be disabled; Tier 0 works without a daemon; Tier 1 works with
  only the shared database; and Tier 2 adapters pass the same semantic suite plus
  their distributed failure cases.
- Complete and Composite hits, variance, privacy, stitching, dependency
  collection, transactional generation advancement, fresh publication reread,
  singleflight fencing, CDN policy, and stale behavior pass deterministic race
  and multi-principal tests.
- The official component catalog covers all specified families with semantic
  HTML, Tailwind CSS 4 theme tokens, documented anatomy and state, keyboard and
  assistive-technology review, browser fixtures, and stable migration policy.
- CLI scaffolds and inspection commands are non-destructive; generated apps
  compile; the dogfood app exercises the supported deployment and interaction
  modes; the manual explains authoring, security, deployment, and recovery.
- Architecture performance budget v1 and checked-in regression baselines pass on
  their recorded environments without disabling correctness, security,
  accessibility, or observability checks.
- Suprnova's targeted checks, browser/runtime checks, workspace lint/test/doc
  gates, generated-project checks, and full release gate pass with no agreed
  item, unresolved drift, placeholder, or undocumented exception remaining.

## Decisions and revisions

- 2026-09-01 -- Benchmark and artifact budgets became on-demand tools outside
  `scripts/gate.sh`; the gate verifies correctness and security only, and no
  artifact carries an absolute ceiling or drift rule.
- 2026-08-30 -- Authorized iteration 005 as the atomic Suprnova integration and
  complete RenderCache implementation. The engine, browser runtime, generated
  artifacts, fixtures, tests, benchmarks, normative specifications, checker, and
  implementation documentation move together under `crates/suprnova-live/`,
  after which the standalone repository is historical rather than a parallel
  maintained authority. The move preserves committed history and carries every
  unresolved iteration-004 qualification as an explicit release blocker.
- 2026-08-26 -- Removed the arbitrary 16 KiB total-size ceiling from the
  asynchronous ESM/classic artifacts. Deterministic builds still report exact
  Brotli bytes and compare each variant with the separately reviewed Task 6
  artifact-size baseline (16,356 ESM and 14,155 classic); drift greater than 15
  percent fails until an explicit version-controlled review updates provenance
  and rationale. Candidate bytes never silently become their own baseline.
- 2026-08-24 -- Rescinded the unsupported 45 KiB absolute core-runtime ceiling.
  Deterministic builds continue to report both core variants, and the exact ESM
  artifact identity keeps transfer changes visible through reviewed benchmark
  rebaselining. A new absolute ceiling requires a functionally complete universal
  core, measured evidence, explicit maintenance headroom, and a recorded rationale;
  optional-artifact and runtime-resource ceilings remain unchanged.
- 2026-08-24 -- Corrected the optional-Stimulus boundary: both the application-
  supplied Stimulus package and Suprnova's bridge/continuity implementation are
  excluded from the universal core and its then-current 45 KiB budget. The bridge
  ships as deterministic ESM/classic adapter artifacts capped at 8 KiB Brotli each,
  registers as one singleton inside the closed optional lifecycle driver, and
  preserves the existing `boot({ stimulus })` contract. Core retains validated,
  ordered morph and island lifecycle edges; bridge failure cannot veto protocol,
  morph, commit, or recovery authority.
- 2026-08-23 -- Locked iteration 004 as the complete standalone upload and
  asynchronous-update foundation. Uploads separate opaque handles from secret
  grants and share lifecycle semantics across daemon-free file and
  provider-neutral direct transport. Typed SSE/WebSocket and polling preserve
  scheduler and HTTP authority, require continuity proof or refresh for current
  status, and never auto-invoke mutating actions. Auxiliary protocols version
  independently rather than manufacturing a generic Live v3. Kept the then-current
  45 KiB universal core cap and assigned manifest-selected upload/async ESM/classic
  artifacts plus `U4/16`, `E100/1K`, and `R100` hard resource budgets so pages
  pay only for declared capabilities.
- 2026-08-22 -- Locked iteration 003 as the complete standalone browser
  interaction runtime across specs 09 through 13. It produces deterministic
  production assets, local and server interaction, scheduling, commit-after-
  morph continuity, and native document enhancement without claiming active
  Suprnova asset/router integration. The work remains in the dedicated
  development workspace because no material integration blocker has appeared.
- 2026-08-21 -- Hardened the iteration 002 server design before implementation:
  protocol v2 carries child/lazy/fresh-render operations while v1 remains
  stable; server-mounted instances have distinct ledger creation; public seed
  state is explicit rather than the default and omitted instance state is
  reconstructed through a fresh mount before verified public values are applied;
  trusted host context replaces a public boolean attestation; and metadata-only
  duplicate recovery refreshes without replay when prior response bytes are
  unavailable.
- 2026-08-21 -- Locked iteration 002 as the standalone server-component kernel.
  Internal Live host adapter contracts permit complete kernel and conformance
  work here, while actual Suprnova router/session/CSRF/auth/tenant adapters and
  public facades remain part of the atomic integration move. Rejected both a
  path dependency on the active checkout and calling a test adapter product
  integration.
- 2026-08-21 -- Adopted the house warning policy: review and resolve Clippy
  findings without blanket `-D warnings`; intentional suppressions must be
  narrowly scoped and carry an explicit reason.
- 2026-08-21 -- Kept initial implementation, normative specifications, and the
  structural checker colocated in the dedicated development workspace. Moving
  into `suprnova/crates/suprnova-live/` is triggered only when separation becomes
  a material blocker, at which point code, specs, and checker move together and
  no parallel maintained authority remains. The large non-normative reference
  catalog stays in the development repository.
- 2026-08-21 -- Completed the Stage 5 technical shape around one internal
  `crates/suprnova-live` engine, the public framework facade, existing macro and
  CLI crates, and a shipped browser runtime; rejected a detached third-party
  crate and a dependency cycle back into `suprnova`.
- 2026-08-21 -- Pinned Askama 0.16, Stimulus 3.2, Idiomorph 0.7.4, and Tailwind
  CSS 4 as the initial implementation references while keeping every
  replaceable dependency behind a Suprnova-owned contract.
- 2026-08-21 -- Adopted strict TypeScript source with shipped ESM and
  classic-script artifacts. Applications do not need a bundler to use Live, and
  optional Stimulus is not part of the core runtime budget.
- 2026-08-21 -- Adopted canonical JSON plus HKDF-SHA-256/HMAC-SHA-256 for
  purpose-separated snapshot integrity; rejected signing serializer-incidental
  bytes or reusing session/CSRF/cache keys directly.
- 2026-08-21 -- Established architecture performance budget v1 with explicit
  workloads, environments, hard caps, and repeatable regression gates.
- 2026-08-21 -- Reference leveling is enabled. Authoritative pinned stack and
  standards sources live under `reference/`; comparative projects remain design
  evidence rather than Suprnova contracts.
