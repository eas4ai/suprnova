# Suprnova Live -- System Overview

Status: Normative design specification
Last revised: 2026-08-21

<!-- The keystone spec, ara2-bridge shape. Stage 2 fills Purpose through
Supported and excluded scope; Stage 5 completes the rest once domains
exist. -->

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

<!-- Completed in Stage 5 after the bounded domains are agreed. -->

## Cross-cutting requirements

<!-- Completed in Stage 5 after the bounded domains are agreed. -->

## Spec map

<!-- Completed in Stage 5 after the bounded domains are agreed. -->

| Spec | Owns |
|---|---|
| _To be completed in Stage 5_ | _Bounded domains are established in Stage 4_ |

## Supported and excluded scope

### Supported

- Canonical server-rendered documents served by real Suprnova routes using
  Askama as the normative checked external-template substrate behind
  Suprnova's view contract.
- Stateful Live component semantics over stateless requests: mounting, typed
  component state, explicitly exposed model binding, registered server
  actions, validation, errors, events, effects, and lifecycle handling.
- Versioned public seed and instanced signed snapshots, first-action promotion,
  an expiring tier-provided instance ledger, one committed outcome per base
  revision, idempotency, expiration, and recovery behavior.
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

<!-- Completed in Stage 5 after the bounded domains are agreed. -->

## System completion criteria

<!-- Completed in Stage 5 after the bounded domains are agreed. -->

## Decisions and revisions

<!-- Completed in Stage 5 after the bounded domains are agreed. -->
