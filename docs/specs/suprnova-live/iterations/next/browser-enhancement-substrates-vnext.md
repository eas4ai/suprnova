# Browser enhancement substrates -- staged for next major version

Status: Staged (not in the current contract and not eligible for the immediate
next iteration)
Captured: 2026-08-24
Version horizon: Next major Suprnova Live version
Target domain: `10-local-reactivity-and-javascript-interop.md`

## What it is

The current Suprnova Live version continues with Stimulus 3.2 as its optional
application-supplied controller substrate and retains the implemented Live local
signal and presentation contracts. No current specification, runtime artifact,
or compatibility claim changes as part of this capture.

The next major-version design cycle shall reconsider the browser-local
enhancement strategy. That discussion shall compare retaining Stimulus,
adopting Alpine's CSP build as a replacement for Stimulus alone, adopting Alpine
as a replacement for both Stimulus and Live's local signal/presentation layer,
and supporting Lit as an optional browser-owned custom-element boundary for
complex widgets. Alpine and Lit are candidates with potentially distinct roles;
this capture does not select either one or commit them to the product.

## Acceptance criteria

- The item remains carried until the next major-version design cycle and shall
  not be promoted into iteration 005 or another intervening iteration merely
  because it is stored in `iterations/next/`.
- Evaluation uses then-current pinned releases and records maintenance health,
  license and provenance, browser support, artifact and retained-memory cost,
  accessibility, developer experience, ecosystem value, and upgrade policy.
- Security review covers strict CSP without requiring `unsafe-eval`, executable
  server-provided strings, unsafe HTML insertion, or browser authority over Live
  state. If Alpine is selected, Suprnova's supported artifact and conformance
  profile use `@alpinejs/csp`.
- The design decides whether browser-local state remains a Suprnova-owned
  contract, becomes Alpine-owned, or supports a compatibility window. It avoids
  shipping two overlapping default reactive models without a documented reason.
- The design preserves real routes, canonical SSR content, Live server
  authority, signed snapshots, per-island scheduling, commit-after-morph
  ordering, typed effects/events, uploads, and asynchronous-update guarantees.
- Morph evaluation proves keyed state retention, focus and form continuity,
  nested-island opacity, cleanup, bfcache behavior, and bounded recovery before
  replacing or augmenting the current guarded Idiomorph adapter.
- Lit evaluation treats client-owned rendering as an explicit custom-element
  ownership boundary. It does not assume experimental JavaScript/Node Lit SSR
  can replace Rust/Askama rendering, and it defines checked attributes or
  properties in, typed events out, theming, Shadow DOM, and Live morph behavior.
- Any adopted change defines migration for existing Stimulus controllers,
  `live:` local directives, application bundles, CSP configuration, official
  components, and ESM/classic asset delivery before compatibility is removed.
- Promotion requires a fresh adversarial architecture review and explicit
  developer confirmation in the next major-version specification process.

## Touches

- Primary owner: `10-local-reactivity-and-javascript-interop.md`.
- Architecture and delivery: `00-overview.md` and
  `09-runtime-bootstrap-and-directives.md`.
- DOM ownership and continuity: `12-dom-morphing-and-identity.md`.
- Checker, browser conformance, CSP, accessibility, artifact, and benchmark
  evidence: `19-developer-tooling-and-testing.md`.
- Application-user behavior and vocabulary: `ux.md` and `glossary.md` if a
  candidate is later selected.
- Official component implications: specs 20 through 25, Tailwind CSS 4 theme
  tokens, catalog fixtures, and complex widget boundaries.
- Current conflict: Stimulus 3.2 and Suprnova-owned local signals are normative
  for the current version. They remain so until one atomic future-version
  decision updates specs, implementation, fixtures, artifacts, and migration
  guidance together.
- Dependency: completion and release of the current version before the next
  major-version design review begins.

## Why not now

The developer explicitly tabled this discussion for the next major Suprnova
Live version so completed Stimulus and local-reactivity work remains coherent
and current iteration scope does not churn.
