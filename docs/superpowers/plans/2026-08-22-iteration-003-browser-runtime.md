# Iteration 003 Browser Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the complete standalone Suprnova Live browser runtime defined by Iteration 003: deterministic framework-owned assets, local interaction, scheduled server interaction, commit-after-morph DOM continuity, and enhanced native navigation.

**Architecture:** Keep the existing Rust/TypeScript protocol, canonical JSON, and compatibility modules as authority boundaries. Add one document runtime with bounded registries and one scheduler per validated island. Generate the closed directive vocabulary from one versioned fixture, bundle Idiomorph 0.7.4 behind a private adapter, leave Stimulus application-supplied, and prove real-DOM behavior through production artifacts served by a standalone test host. No code or path dependency crosses into Suprnova or Magnetar.

**Tech Stack:** Rust 1.91.1, strict TypeScript 6.0.3, Vitest 4.1.11, fast-check 4.9.0, esbuild 0.28.2, Idiomorph 0.7.4, Stimulus 3.2.2 as a test-only optional integration, Playwright 1.62.1, axe-core 4.13.0, Web Crypto, native DOM/History/View Transition/Speculation Rules APIs.

---

## Execution rules

- Work only in `/home/shawn/workspace2/suprnova-live/.worktrees/iteration-003-browser-runtime` on branch `iteration-003-browser-runtime`.
- Start every shell command with `rtk`; use `rtk proxy` for a raw subordinate command.
- Use `apply_patch` for hand edits. Formatting and deterministic generators may rewrite their owned outputs.
- Follow red/green/refactor for every behavior change: add the smallest failing test, run it and record the expected failure, implement the smallest complete behavior, rerun the focused test, then run the neighboring suite before committing.
- Use injected clocks, randomness, fetch, observers, animation completion, and navigation ports. Correctness tests must not depend on elapsed sleeps.
- Do not use blanket `-D warnings`. Review Clippy output directly.
- Do not modify, format, generate into, depend on, commit in, or push from `/home/shawn/workspace2/suprnova` or `/home/shawn/workspace2/suprnova-magnetar`.
- `agent-browser` is for implementation-time exploration and accessibility inspection. DevTools MCP is for profiling, lifecycle, network, memory, observer, and bfcache diagnosis. Neither replaces checked-in Playwright or benchmark evidence.
- Make each task's commit locally. Never push this branch.

## Task 1: Convert the browser package into a pinned production workspace

**Files:**

- Modify: `browser/package.json`
- Modify: `browser/package-lock.json`
- Modify: `browser/tsconfig.json`
- Modify: `browser/tsconfig.build.json`
- Modify: `browser/eslint.config.mjs`
- Modify: `browser/.gitignore`
- Create: `browser/tests/package-contract.test.ts`
- Create: `browser/src/version.ts`
- Modify: `browser/src/index.ts`

- [ ] Add a failing package-contract test that reads `package.json` and asserts the production name, exact tool versions, exact Idiomorph pin, core/runtime entry points, and the complete script set. It must also assert that Stimulus is absent from `dependencies` and present only in `devDependencies` for bridge conformance.
- [ ] Run `rtk npm --prefix browser test -- package-contract.test.ts`; record failure because the production workspace contract is not present.
- [ ] Install exact versions without ranges:

  ```bash
  rtk npm --prefix browser install --save-exact idiomorph@0.7.4
  rtk npm --prefix browser install --save-dev --save-exact esbuild@0.28.2 @playwright/test@1.62.1 axe-core@4.13.0 fast-check@4.9.0 @hotwired/stimulus@3.2.2
  ```

- [ ] Change the package identity to `@suprnova/live`, retain `private: true` while this is a development workspace, and define these scripts exactly: `generate`, `generate:check`, `format`, `format:check`, `lint`, `typecheck`, `test:unit`, `test:browser`, `test:browser:install`, `test`, `build`, `build:check`, `budget`, and `compatibility:check`. `test` runs unit tests only so focused TDD remains fast; the root gate invokes all categories explicitly.
- [ ] Establish the version boundary in `browser/src/version.ts`:

  ```ts
  export const ENGINE_VERSION = "0.1.0";
  export const RUNTIME_CONTRACT_VERSION = 1 as const;
  export const SUPPORTED_SNAPSHOT_VERSIONS = [1] as const;
  export const SUPPORTED_PROTOCOL_VERSIONS = [1, 2] as const;

  export type SupportedProtocolVersion = (typeof SUPPORTED_PROTOCOL_VERSIONS)[number];
  ```

  Re-export it from `index.ts`; update the old Iteration 002 wording without changing protocol or snapshot numbers.
- [ ] Keep DOM libraries in `lib`, Node/Vitest/Playwright types scoped to their configs, include `src`, `tests`, and `e2e` deliberately, and make generated production output, browser downloads, traces, screenshots, and local evidence ignored without ignoring reviewed baselines.
- [ ] Run package test, format check, lint, and typecheck.
- [ ] Commit: `build(browser): establish pinned runtime workspace`.

## Task 2: Add the shared Iteration 003 fixture catalog and complete response order

**Files:**

- Create: `fixtures/v3/directive-grammar.json`
- Create: `fixtures/v3/runtime-config.json`
- Create: `fixtures/v3/island-metadata.json`
- Create: `fixtures/v3/scheduling.json`
- Create: `fixtures/v3/response-application.json`
- Create: `fixtures/v3/morph-identity.json`
- Create: `fixtures/v3/diagnostics.json`
- Create: `fixtures/v3/compatibility.json`
- Create: `fixtures/v3/navigation.json`
- Create: `fixtures/v3/manifest.sha256`
- Modify: `src/conformance.rs`
- Modify: `src/protocol/ordering.rs`
- Modify: `browser/src/conformance.ts`
- Modify: `browser/src/ordering.ts`
- Modify: `browser/tests/golden-fixtures.test.ts`
- Modify: `tests/golden_fixtures.rs`
- Modify: `tests/response_ordering.rs`

- [ ] Add failing Rust and TypeScript tests requiring fixture version 3, exact manifest parity, and a response-plan case for each of: v1 redirect, v2 navigated, HTML success, no-render success, reflected URL, signed child delivery, both child plus reflection, failed morph recovery, rejected, refresh, and fatal.
- [ ] Run the focused Rust and browser tests; record failure because v3 and the new phases are unknown.
- [ ] Extend the semantic step vocabulary in both languages without changing the v1 fixture:

  ```rust
  pub enum ApplicationStep {
      Navigate,
      PreflightMorph,
      Morph,
      ValidateNoRender,
      CommitSnapshotAndRevision,
      ReconcileModelsAndValidation,
      RestoreFocus,
      QueueChildDeliveries,
      ReflectUrl,
      DispatchEvents,
      RunRegisteredEffects,
      SettleFeedback,
      RetainDom,
      RequestFreshRenderWithoutReplay,
      RequestFreshIsland,
      StopLive,
  }
  ```

  Add `application_plan_v2(&UpdateResponseV2, MorphDisposition)` rather than changing the stable v1 function. In v2, redirect and `UrlIntent::Navigated` return only `Navigate`; nonterminal accepted plans place child delivery and reflection after commit/focus and before events/effects.
- [ ] Replace the TypeScript positional planner with a closed discriminated input while retaining a v1 compatibility wrapper:

  ```ts
  export interface ApplicationPlanInput {
    readonly protocol: 1 | 2;
    readonly outcome: "accepted" | "duplicate" | "rejected" | "refresh_required" | "fatal";
    readonly render: "redirect" | "navigated" | "html" | "no_render" | "none";
    readonly morph: "not_attempted" | "succeeded" | "failed_after_acceptance";
    readonly hasChildDeliveries: boolean;
    readonly hasReflectedUrl: boolean;
    readonly recovery: "retain_dom" | "retry" | "refresh_island" | "remount_island" | "navigate" | "stop" | null;
  }
  ```

- [ ] Keep `fixtures/v1` and `fixtures/v2` authoritative for the wire protocol. Version 3 describes browser behavior and references protocol cases; it must not copy or relax the v2 envelope schema.
- [ ] Generate the v3 manifest with the existing canonical manifest algorithm and add it to the fixture catalog in deterministic filename order.
- [ ] Run both focused suites, all golden fixtures, and `rtk git diff --check`.
- [ ] Commit: `test: add shared browser-runtime conformance fixtures`.

## Task 3: Generate one closed directive contract for Rust and TypeScript

**Files:**

- Create: `scripts/generate-browser-contracts.mjs`
- Create: `src/checker/generated_directive_contract.rs`
- Modify: `src/checker/mod.rs`
- Modify: `src/checker/directive.rs`
- Create: `browser/src/generated/directive-contract.ts`
- Create: `browser/src/directives/types.ts`
- Create: `browser/src/directives/parser.ts`
- Create: `browser/tests/directive-grammar.test.ts`
- Modify: `tests/checker_positive.rs`
- Modify: `tests/checker_negative.rs`
- Create: `tests/fixtures/checker/pass/iteration-003-directives.html`
- Create: `tests/fixtures/checker/fail/iteration-003-directives.html`

- [ ] Add failing parity tests that enumerate every directive, target kind, literal kind, argument form, modifier, conflict, and fallback from `fixtures/v3/directive-grammar.json` through the Rust checker and browser parser. Include unknown names, repeated modifiers, invalid timing, dynamic structure, unsafe target names, and incompatible combinations.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --test checker_positive --test checker_negative` and `rtk npm --prefix browser test -- directive-grammar.test.ts`; record the mismatches with the current handwritten checker switch.
- [ ] Lock the iteration vocabulary in the fixture by category: action/model events; local signal and presentation; feedback targets; effect and public-call lookup; keys and nested ownership; preserve/ignore/replace/persist/teleport; transition; lazy completion; and native navigation/prefetch. Mark upload, poll, stream, and server-push forms as reserved outside Iteration 003 rather than silently implementing them.
- [ ] Make the public names explicit rather than leaving discovery to the implementer. The fixture contains exactly these in-scope names:

  ```text
  click submit change input keydown init model url
  signal toggle show class attr selected expanded inert focus
  idle dirty queued loading validating success interrupted offline retrying error
  effect on call
  component key lazy
  preserve ignore replace persist teleport transition
  navigate prefetch
  ```

  Event modifiers are the closed prevent/stop/once/self/trusted/capture and key-filter set. Model modifiers are immediate/change/blur/action/submit plus bounded debounce/throttle and scheduler-policy forms. Feedback modifiers select show/hide, checked class, disabled, busy, and live-region presentation. Morph, transition, and navigation modifiers select only their fixture-enumerated modes. `poll`, `stream`, upload/progress, and arbitrary timing/name suffixes are reserved and inert in this iteration.
- [ ] Generate equivalent closed descriptors:

  ```ts
  export interface DirectiveContract {
    readonly name: string;
    readonly owner: "island" | "keyed_scope" | "element";
    readonly value: "empty" | "identifier" | "literal" | "field" | "action" | "target" | "mapping";
    readonly modifiers: readonly string[];
    readonly conflicts: readonly string[];
    readonly phase: "local" | "schedule" | "feedback" | "morph" | "navigation";
  }
  ```

  The Rust generated form uses enums and static slices, not runtime JSON parsing. Both generated files contain the v3 fixture manifest hash and a generated-file provenance header.
- [ ] Make generation deterministic and add `--check` mode that computes expected bytes in memory and exits nonzero on drift. The generator may rewrite only its two declared generated outputs.
- [ ] Refactor `validate_directive` to consult the generated contract first, then apply component-specific action/model/event/effect checks. Preserve source spans, branch awareness, accessible-click checks, and the `DynamicStructureUnproved` distinction.
- [ ] Implement a bounded browser parser returning `ParsedDirective | DirectiveDiagnostic`; it never evaluates values, compiles expressions, constructs module URLs, or reads an endpoint from element markup.
- [ ] Run generator check, Rust checker suites, TypeScript unit tests, formatting, lint, and typecheck.
- [ ] Commit: `feat: share the closed Live directive grammar`.

## Task 4: Implement bounded runtime configuration and redacted diagnostics

**Files:**

- Create: `browser/src/runtime/types.ts`
- Create: `browser/src/runtime/limits.ts`
- Create: `browser/src/runtime/ports.ts`
- Create: `browser/src/runtime/diagnostics.ts`
- Create: `browser/src/runtime/config.ts`
- Create: `browser/tests/runtime-config.test.ts`
- Create: `browser/tests/diagnostics.test.ts`

- [ ] Add failing table and property tests for missing/duplicate config, excessive bytes/depth/entries, unknown fields, unsupported versions, unsafe schemes, protocol-relative/backslash/control-character URLs, cross-origin endpoints without host approval, invalid credentials/timeouts/concurrency, DOM-requested verbose diagnostics, and attempted diagnostic leakage.
- [ ] Run the focused Vitest files; record failure because the runtime foundation does not exist.
- [ ] Define the trusted bootstrap boundary separately from inert document input:

  ```ts
  export interface BootstrapOptions {
    readonly document?: Document;
    readonly allowedEndpointOrigins?: readonly string[];
    readonly diagnostics?: "off" | "errors" | "verbose";
    readonly clock?: RuntimeClock;
    readonly randomness?: RuntimeRandomness;
    readonly transport?: TransportPort;
    readonly navigation?: NavigationPort;
  }

  export interface RuntimeConfig {
    readonly runtimeContractVersion: 1;
    readonly protocol: Readonly<{ minimum: 1 | 2; maximum: 1 | 2 }>;
    readonly endpoint: URL;
    readonly credentials: "same-origin" | "include";
    readonly requestTimeoutMs: number;
    readonly maxResponseBytes: number;
    readonly maxQueuedPerIsland: number;
    readonly maxParallelPerIsland: number;
    readonly assetIdentity: string;
  }
  ```

  Same-origin requires no application JavaScript. Cross-origin endpoints require an origin supplied through `BootstrapOptions`; a document element can never approve itself.
- [ ] Use one `application/json` configuration element with an exact ID and exact key set. Parse text with existing canonical/shape helpers and hard bounds before creating URLs or arrays. Return source-attributed stable codes, never raw JSON or endpoint text.
- [ ] Define a closed diagnostic record with code, severity, safe phase, fixture-stable detail code, and bounded monotonic sequence. Redaction must exclude snapshots, signatures, cookies, tokens, model/transient values, HTML, arbitrary URL strings, instance/correlation/idempotency identities, and exception messages.
- [ ] Inject production ports for clock, randomness, fetch, navigation, observer creation, scheduling, reduced motion, and feature detection. Tests use deterministic ports; production defaults use platform APIs.
- [ ] Run focused tests plus `rtk npm --prefix browser run lint` and `typecheck`.
- [ ] Commit: `feat(browser): add bounded configuration and diagnostics`.

## Task 5: Build reproducible ESM and classic assets with a typed manifest

**Files:**

- Create: `browser/src/bootstrap.ts`
- Create: `browser/src/entry-esm.ts`
- Create: `browser/src/entry-classic.ts`
- Create: `browser/src/global.d.ts`
- Create: `browser/src/assets.ts`
- Create: `browser/src/runtime/runtime.ts`
- Create: `browser/scripts/build.mjs`
- Create: `browser/scripts/check-build.mjs`
- Create: `browser/tests/build-contract.test.ts`
- Modify: `browser/package.json`
- Modify: `browser/tsconfig.build.json`
- Modify: `browser/.gitignore`

- [ ] Add a failing build-contract test for exact output names, equivalent version/protocol exports, content types, preload/module intent, cache metadata, SHA-256/SRI hashes, omitted production maps, bundled Idiomorph identity, stable timestamps/ordering, and a second-build byte comparison.
- [ ] Run the test and `rtk npm --prefix browser run build`; record failure because no production bundling contract exists.
- [ ] Implement the singleton storage key and public surface:

  ```ts
  export const RUNTIME_SYMBOL = Symbol.for("suprnova.live.runtime.v1");

  export interface SuprnovaLivePublicApi {
    readonly version: string;
    boot(options?: BootstrapOptions): RuntimeHandle;
  }
  ```

  The ESM file exports this API. The classic file defines one non-replaceable `window.SuprnovaLive` facade. Both call the same core bootstrap and use the same symbol, so loading both cannot duplicate the runtime. Tasks 9 and 10 extend the same facade with fully implemented effect/call registration and Stimulus attachment when those subsystems exist; Task 5 does not expose inert methods.
- [ ] Use esbuild with pinned options, stable banner, no wall-clock/build-path content, legal comments retained in one deterministic location, `target: ["chrome111", "edge111", "firefox128", "safari16.4"]`, and production source maps off unless an explicit development build flag is passed.
- [ ] Emit `dist/suprnova-live.esm.js`, `dist/suprnova-live.classic.js`, `dist/index.d.ts`, and `dist/suprnova-live.assets.json`. Define the manifest in `assets.ts` with runtime/protocol/snapshot versions, bytes, SHA-256, SRI, content type, script kind, preload relation, immutable cache policy, and Idiomorph provenance.
- [ ] Make `build:check` run two builds in separate temporary directories and compare every production byte and manifest entry. Keep build output untracked but require it in later test and budget steps.
- [ ] Verify both artifacts parse under their intended script modes, no application bundler is required, and production output has no source map reference, `eval`, `new Function`, or dynamic module URL.
- [ ] Run unit/build contract, build, build check, typecheck, and bundle inspection.
- [ ] Commit: `build(browser): emit deterministic runtime assets`.

## Task 6: Add the standalone real-browser host and first connected island

**Files:**

- Create: `browser/playwright.config.ts`
- Create: `browser/test-host/server.mjs`
- Create: `browser/test-host/scenarios.mjs`
- Create: `browser/test-host/certificates/README.md`
- Create: `browser/e2e/support/runtime-page.ts`
- Create: `browser/e2e/bootstrap.spec.ts`
- Create: `browser/e2e/csp.spec.ts`
- Create: `browser/src/islands/metadata.ts`
- Create: `browser/src/islands/snapshot-view.ts`
- Create: `browser/src/islands/record.ts`
- Create: `browser/src/islands/discovery.ts`
- Modify: `browser/src/runtime/runtime.ts`
- Modify: `browser/src/bootstrap.ts`
- Modify: `src/mount/service.rs`
- Modify: `src/mount/mod.rs`
- Create: `src/mount/public.rs`
- Create: `src/view/root.rs`
- Modify: `src/view/mod.rs`
- Modify: `tests/initial_mount.rs`
- Create: `tests/public_seed_mount.rs`

- [ ] Add failing Playwright cases for initial SSR visibility before startup, one valid instanced island, one seed island with no request, malformed and duplicate roots, incompatible protocol, duplicate ESM/classic loading, nonce/hash CSP, blocked runtime, and dynamic server-origin insertion/removal. The failed-runtime cases must leave initial content exposed.
- [ ] Run `rtk npm --prefix browser run test:browser -- bootstrap.spec.ts csp.spec.ts`; record failure because the host and runtime connection do not exist.
- [ ] Refactor the engine-owned wrapper into one `view::root` assembler used by private initial mounts, public seed mounts, and later accepted successor renders. Its typed input carries component, slot, document key, runtime/protocol contract, snapshot form, signed envelope, revision, optional instance identity, and bounded flags. Update private mount tests before changing production output.
- [ ] Add a public seed mount output that signs only verified public seed state, emits the same root shape with `snapshot-form="seed"`, revision zero, and no instance identity, and creates no ledger record. Prove in Rust that rendering/discovery metadata has no promotion nonce and no server instance allocation.
- [ ] Parse one exact root shape:

  ```ts
  export interface IslandMetadata {
    readonly component: string;
    readonly slot: string;
    readonly documentKey: string;
    readonly protocolMinimum: 1 | 2;
    readonly runtimeContract: 1;
    readonly snapshot: Readonly<Record<string, unknown>>;
    readonly snapshotForm: "seed" | "instance";
    readonly instanceId: string | null;
    readonly revision: bigint;
    readonly lazyComplete: boolean;
  }
  ```

  Validate exact element ownership, one root marker, bounded attributes, safe identities, snapshot encoding, form/revision consistency, endpoint association through document config only, compatibility, and document-local uniqueness before allocating a record.
- [ ] Decode a bounded read-only view of the signed envelope solely to correlate public metadata such as form, component, slot, instance, and revision. The browser does not verify the HMAC and the decoded view is never authority; server verification remains mandatory. Reject root/envelope disagreement as incompatible before connection.
- [ ] Implement one `DocumentRuntime` with one `MutationObserver`, a bounded delegated-listener registry, maps keyed by element and document identity, deterministic document-order discovery, and idempotent `start`, `suspend`, `resume`, and `dispose` methods.
- [ ] Mutation processing sends every inserted candidate through the same bounded root validation path. A successfully parsed record still grants no authority: signed snapshots and current authorization remain server-verified on every intent. Invalid or copied/cross-bound roots fail locally or at the server without letting DOM attributes mint authority. Nested valid roots become independent records and are opaque to parent directive scans.
- [ ] Make every record own a disposal stack. Removal/replacement retires scheduler placeholders, timers, observer registrations, signals, controllers, and extension resources exactly once even when observer records repeat.
- [ ] Serve only `dist` assets, fixture-driven HTML, and test endpoint responses from the host. Label it conformance apparatus in code and diagnostics; it does not emulate Suprnova middleware or claim framework integration.
- [ ] Run Playwright Chromium/Firefox/WebKit focused cases, Rust mount tests, unit tests, and build check.
- [ ] Commit: `feat(browser): bootstrap and discover Live islands`.

## Task 7: Route delegated events, lazy completion, and seed first intent

**Files:**

- Create: `browser/src/directives/ownership.ts`
- Create: `browser/src/directives/events.ts`
- Create: `browser/src/directives/modifiers.ts`
- Create: `browser/src/islands/nonce.ts`
- Create: `browser/src/islands/lazy.ts`
- Create: `browser/src/scheduler/intent.ts`
- Create: `browser/tests/event-routing.test.ts`
- Create: `browser/tests/seed-nonce.test.ts`
- Create: `browser/e2e/directives.spec.ts`
- Create: `browser/e2e/nested-islands.spec.ts`
- Create: `browser/e2e/seed-and-lazy.spec.ts`
- Modify: `browser/src/runtime/runtime.ts`

- [ ] Add failing unit/property/browser cases for composed paths, nearest owner, nested exclusion, open and closed shadow roots, disabled controls, trusted-event policy, capture/bubble phase, native default, prevent/stop/once/self/key filters, keyboard activation, repeated activation, removal during dispatch, and bounded directive values.
- [ ] Add failing seed/lazy cases proving discovery and local events make no request, the first server intent obtains exactly 16 cryptographic bytes or more, the nonce is stable across only that intent's allowed retries, a second intent gets a new nonce, missing Web Crypto fails closed, and repeated lazy activation queues one `lazy_complete` operation per surviving identity.
- [ ] Run focused Vitest and Playwright tests; record failure before adding listeners or server-intent construction.
- [ ] Define an intent source without executable markup:

  ```ts
  export interface IntentSource {
    readonly island: IslandRecord;
    readonly element: Element;
    readonly directive: ParsedDirective;
    readonly eventType: string;
    readonly trusted: boolean;
  }

  export type ServerOperation =
    | Readonly<{ kind: "sync_model"; field: string }>
    | Readonly<{ kind: "invoke_action"; name: string; arguments: Readonly<Record<string, JsonValue>> }>
    | Readonly<{ kind: "params_changed" }>
    | Readonly<{ kind: "lazy_complete" }>
    | Readonly<{ kind: "fresh_render" }>;
  ```

- [ ] Install only the event types and phases present in the generated grammar, once per document. Resolve `event.composedPath()` to one validated directive and its nearest connected island; crossing a nested root ends parent resolution. Apply modifiers in the fixture-defined order and preserve native behavior unless `prevent` is explicit and valid.
- [ ] Generate promotion nonces through the injected randomness port backed by `crypto.getRandomValues`. Encode base64url without padding. Store the nonce on the immutable scheduler intent, not the island, and erase the reference after accepted, terminal, canceled, or exhausted resolution.
- [ ] Implement lazy activation through an injected intersection/activation port. Stable identity owns a `pending | queued | resolved | retired` marker; discovery and post-morph scans cannot enqueue duplicates. Removal uses ordinary intent cancellation and record disposal.
- [ ] Use bounded WeakMap/WeakSet provenance for directives parsed during initial connection or a validated insertion/morph scan. Attribute-only mutation after a node was scanned does not become executable until the owning runtime deliberately revalidates that subtree; even then, DOM data remains a proposal and cannot mint snapshot/action authority.
- [ ] Run focused tests across all three Playwright engines, then format/lint/typecheck.
- [ ] Commit: `feat(browser): route owned Live directives`.

## Task 8: Implement typed local signals and accessible presentation

**Files:**

- Create: `browser/src/signals/value.ts`
- Create: `browser/src/signals/scope.ts`
- Create: `browser/src/signals/graph.ts`
- Create: `browser/src/signals/presentation.ts`
- Create: `browser/src/signals/lifecycle.ts`
- Create: `browser/tests/signals.test.ts`
- Create: `browser/tests/presentation.test.ts`
- Create: `browser/e2e/local-signals.spec.ts`
- Modify: `browser/src/runtime/runtime.ts`

- [ ] Add failing tests for boolean/string/integer/null literals, malformed literals, duplicate declaration, deterministic shadowing, missing/circular references, batching, same-value suppression, island leakage, local interaction without fetch, initial SSR mismatch, hidden/ARIA/inert/focus semantics, unsafe class/attribute names, reduced motion, and exact disposal.
- [ ] Run focused unit and browser cases; record failure because local state is not implemented.
- [ ] Implement only literal, typed state:

  ```ts
  export type SignalValue = boolean | string | number | null;

  export interface SignalScope {
    readonly identity: string;
    get(name: string): SignalValue;
    set(name: string, value: SignalValue): void;
    toggle(name: string): void;
    reset(name?: string): void;
    batch(update: () => void): void;
    dispose(): void;
  }
  ```

  A scope belongs to one island root or one checked keyed local root. Parent lookup may follow declared lexical ancestry inside that island only. There is no expression parser, arbitrary property path, global store, local storage, session storage, snapshot field, or durability adapter.
- [ ] Build an explicit dependency graph from parsed presentation directives. Flush changed targets once per microtask through the injected scheduling port and use stable document order. Prevent cycles and cap declarations, edges, affected targets, and flush work.
- [ ] Apply the fixture-locked targets for visibility, toggle, class, attribute, selected, expanded, inert, and focus. Class/attribute names must occur in the checked directive contract; URL/event-handler/style/module names are forbidden. Visibility updates `hidden`, relevant ARIA, inertness, and focusability coherently.
- [ ] Compare initial signal-derived presentation with SSR output without hiding initial content on failure. Mismatch emits a safe diagnostic and applies the contract's deterministic reconcile mode.
- [ ] Expose capture/restore hooks keyed by island and local-scope identity. Do not transfer values yet; Task 18 integrates these hooks with the morph transaction.
- [ ] Run focused tests, accessibility assertions, all Playwright engines, format/lint/typecheck.
- [ ] Commit: `feat(browser): add local signals and presentation`.

## Task 9: Add bounded effects, public calls, and optimistic projection

**Files:**

- Create: `browser/src/extensions/schema.ts`
- Create: `browser/src/extensions/registry.ts`
- Create: `browser/src/extensions/effects.ts`
- Create: `browser/src/extensions/calls.ts`
- Create: `browser/src/extensions/projection.ts`
- Create: `browser/tests/extensions.test.ts`
- Create: `browser/tests/projection.test.ts`
- Create: `browser/e2e/effects.spec.ts`
- Modify: `browser/src/bootstrap.ts`

- [ ] Add failing cases for valid registration/disposal, duplicate name/version, unknown effect/call, wrong payload schema, excessive payload, wrong island/scope/phase, throwing handler, late completion after disposal, code string, module URL, snapshot/revision mutation, and projection accept/reject/timeout/cancel/remove behavior.
- [ ] Run focused tests and record the missing registries.
- [ ] Define closed registrations:

  ```ts
  export interface EffectRegistration {
    readonly name: string;
    readonly version: number;
    readonly schema: PayloadSchema;
    readonly phase: "after_commit";
    run(context: EffectContext, payload: JsonValue): void | Promise<void>;
  }

  export interface RuntimeCallRegistration {
    readonly name: string;
    readonly input: PayloadSchema;
    readonly output: PayloadSchema;
    run(context: RuntimeCallContext, input: JsonValue): JsonValue | Promise<JsonValue>;
  }
  ```

  Store registrations on the document runtime, duplicate-check `(name, version)`, and return idempotent disposables. The context exposes safe island-local calls only; it has no snapshot, signature, raw response, endpoint mutation, arbitrary selector, or module loader.
- [ ] Validate payload shape/depth/entries/string bytes before invoking user code. Normalize failures to closed diagnostics, continue remaining effects according to the v3 fixture, and never roll back or conceal an already committed response.
- [ ] Apply an injected bounded effect deadline and lifecycle epoch to every asynchronous handler. A timeout, cancellation, or late settlement becomes one scoped failure and cannot postpone final feedback, navigation, cleanup, or later response application indefinitely.
- [ ] Implement optimistic projection as a reversible patch list tied to one immutable intent identity. Projection state is explicitly `pending`; it can alter only presentation targets declared by the originating directive. Accepted HTML wins; rejection/interruption/cancellation rolls back; incompatibility requests recovery.
- [ ] Ensure public calls enter the owning island scheduler for server work or the owning signal scope for local work. They cannot bypass directive grammar, policy, proposal tracking, or action registration by fabricating a transport envelope.
- [ ] Run focused suites, browser cases, format/lint/typecheck.
- [ ] Commit: `feat(browser): add safe Live extension registries`.

## Task 10: Integrate an application-supplied Stimulus 3.2 lifecycle

**Placement correction (2026-08-24):** “Stimulus-free core” excludes both the
application-supplied package and Suprnova's bridge/continuity implementation.
The public structural ports and unchanged `boot({ stimulus })` options remain
stable, but `stimulus/bridge.ts` and `stimulus/lifecycle.ts` are built only into
deterministic optional ESM/classic adapter artifacts. Core owns validated,
ordered morph and island lifecycle events through its closed driver seam; the
adapter owns application/definition validation and continuity records. The
adapter must register before boot, duplicate registration is idempotent, and a
missing/incompatible adapter reports a bounded Stimulus-only diagnostic while
ordinary Live remains operational.

**Files:**

- Create: `browser/src/stimulus/port.ts`
- Create: `browser/src/stimulus/bridge.ts`
- Create: `browser/src/stimulus/lifecycle.ts`
- Create: `browser/tests/stimulus-bridge.test.ts`
- Create: `browser/e2e/stimulus.spec.ts`
- Modify: `browser/src/bootstrap.ts`
- Modify: `browser/tests/build-contract.test.ts`

- [ ] Add failing unit and real-browser cases with the actual test-only Stimulus 3.2.2 `Application`: initial connect, preserved keyed root, inserted root, removed root, forced replacement, repeated morph, controller throw, bridge detach/reattach, nested island ownership, and document disposal.
- [ ] Add build assertions that core metafiles contain neither
  `stimulus/bridge.ts`, `stimulus/lifecycle.ts`, `@hotwired/stimulus`, nor its
  controller implementation; adapter metafiles also exclude
  `@hotwired/stimulus`. Preserve production ESM/classic integration tests.
- [ ] Run focused tests and record failure because no bridge exists.
- [ ] Define the structural public port rather than importing Stimulus in production:

  ```ts
  export interface StimulusApplicationPort {
    start(): void;
    stop(): void;
    load(...definitions: readonly unknown[]): void;
  }

  export interface StimulusMorphBridge {
    beforeMorph(scope: Element): StimulusContinuity;
    afterMorph(continuity: StimulusContinuity, scope: Element): void;
    disposeScope(scope: Element): void;
    dispose(): void;
  }
  ```

  The adapter coordinates through DOM identity and public Stimulus lifecycle. It must not patch Stimulus internals, replace MutationObserver globally, or expose Idiomorph callbacks as application API.
- [ ] Preserve controller elements by stable Live identity; let existing connected nodes remain connected. Inserted/replaced identities follow ordinary Stimulus observation, removed identities disconnect once, and bridge failure emits a scoped diagnostic without bypassing morph safety or response ordering.
- [ ] Run focused unit and Chromium/Firefox/WebKit cases, build contract, and typecheck.
- [ ] Commit: `feat(browser): bridge application Stimulus lifecycle`.

## Task 11: Implement the bounded per-island scheduler

**Files:**

- Create: `browser/src/scheduler/types.ts`
- Create: `browser/src/scheduler/policy.ts`
- Create: `browser/src/scheduler/state.ts`
- Create: `browser/src/scheduler/scheduler.ts`
- Create: `browser/src/scheduler/disposition.ts`
- Create: `browser/tests/scheduler.test.ts`
- Create: `browser/tests/scheduler-properties.test.ts`
- Create: `browser/e2e/multiple-islands.spec.ts`
- Modify: `browser/src/islands/record.ts`

- [ ] Add failing deterministic state-machine and fast-check command-model tests for FIFO, queue limit, duplicate suppression, cancel pending, replace pending, latest only, safe parallel, in-flight cancellation, response application serialization, retirement, repeated callbacks, and independence across two or more islands.
- [ ] Run focused tests and record failure because island records have no scheduler.
- [ ] Lock scheduler state and policy effects:

  ```ts
  export type SchedulerPolicy =
    | Readonly<{ kind: "fifo" }>
    | Readonly<{ kind: "replace_pending"; key: string }>
    | Readonly<{ kind: "drop_duplicate"; key: string }>
    | Readonly<{ kind: "latest_only"; key: string; abortInFlight: boolean }>
    | Readonly<{ kind: "parallel"; group: string; maximum: number }>;

  export type IntentDisposition =
    | "accepted"
    | "rejected"
    | "duplicate"
    | "canceled"
    | "superseded"
    | "stale"
    | "out_of_order"
    | "incompatible"
    | "retired";
  ```

  FIFO is the default. Every alternate policy declares effects separately for unsent work, transport abort, and application eligibility. Parallel transport is allowed only for fixture-declared commutative work; response application remains serialized against accepted revision.
- [ ] Bound queued count, in-flight count, completed-disposition retention, recovery count, and callback work. Overflow yields a closed local rejection without evicting an already in-flight authoritative request.
- [ ] Retiring a scheduler aborts permitted transport and marks all future callbacks ineligible. It cannot claim server rollback, erase a committed response, or apply after island removal/navigation.
- [ ] Run state-machine/property/browser tests and leak-sensitive disposal assertions.
- [ ] Commit: `feat(browser): schedule island work deterministically`.

## Task 12: Synchronize models without overwriting newer browser edits

**Files:**

- Create: `browser/src/models/value.ts`
- Create: `browser/src/models/control.ts`
- Create: `browser/src/models/state.ts`
- Create: `browser/src/models/timing.ts`
- Create: `browser/src/models/forms.ts`
- Create: `browser/tests/model-state.test.ts`
- Create: `browser/tests/model-timing.test.ts`
- Create: `browser/e2e/models-and-forms.spec.ts`
- Modify: `browser/src/directives/events.ts`
- Modify: `browser/src/scheduler/intent.ts`

- [ ] Add failing cases for missing versus null, text/number/checkbox/radio/select/multi-select, malformed control mapping, dirty/accepted/validation separation, immediate/change/blur/debounce/throttle/action/submit, scoped timers, newest submit value exactly once, stale timer suppression, older accepted response with newer edit, reset, disabled controls, nested ownership, and file-input exclusion.
- [ ] Run focused tests and record the absence of model tracking.
- [ ] Store four distinct layers per field:

  ```ts
  export interface ModelFieldState {
    readonly field: string;
    readonly browserProposal: JsonValue | Missing;
    readonly acceptedServerValue: JsonValue | Missing;
    readonly validation: readonly ValidationIssue[];
    readonly inFlightIntent: string | null;
    readonly editSequence: bigint;
  }
  ```

  `dirty` is derived from proposal versus accepted value and never from loading or validation. File controls return a distinct unsupported-for-JSON result and keep browser ownership for the later upload iteration.
- [ ] Implement timing with the injected monotonic clock and scheduler port. Debounce and throttle keys include island, field, directive identity, and timing policy. Superseded unsent proposals and timer callbacks are removed; no stale callback can enqueue after submit or disposal.
- [ ] On submit, synchronously sample every eligible control owned by the form's island, cancel obsolete field timers, construct one ordered sync/action batch, and allow native invalid/form semantics according to the directive contract. Ordinary forms without Live directives remain untouched.
- [ ] Reconcile accepted responses by edit sequence: advance accepted server state and clear matching validation while retaining a newer unsent browser proposal and its control presentation.
- [ ] Run unit/property/browser tests across all engines and keyboard paths.
- [ ] Commit: `feat(browser): synchronize Live models and forms`.

## Task 13: Build protocol transport, interruption, and safe retry

**Files:**

- Create: `browser/src/transport/request.ts`
- Create: `browser/src/transport/response.ts`
- Create: `browser/src/transport/fetch.ts`
- Create: `browser/src/transport/retry.ts`
- Create: `browser/src/transport/state.ts`
- Create: `browser/tests/request-builder.test.ts`
- Create: `browser/tests/transport.test.ts`
- Create: `browser/tests/retry.test.ts`
- Create: `browser/e2e/network.spec.ts`
- Modify: `browser/src/scheduler/scheduler.ts`
- Create: `src/protocol/browser_context.rs`
- Modify: `src/protocol/mod.rs`
- Modify: `src/component/instance.rs`
- Modify: `src/execution/service.rs`
- Modify: `crates/suprnova-live-test-support/src/harness.rs`
- Modify: `tests/seed_promotion.rs`
- Modify: `tests/execution_concurrency.rs`
- Modify: `tests/execution_fault_matrix.rs`
- Modify: `tests/execution_order.rs`
- Modify: `benches/action_framework_budget.rs`
- Create: `tests/browser_render_context.rs`

- [ ] Add failing cross-fixture cases constructing every v1/v2 request form, including instanced, seed promotion, params changed, lazy complete, and fresh render. Add failures for excessive operations/proposals/extensions, wrong protocol, unsafe endpoint, wrong media/status/size, timeout, abort, offline, changed retry semantics, and correlation misuse.
- [ ] Run focused tests and record failure before implementing request serialization and fetch.
- [ ] Define immutable transport identity:

  ```ts
  export interface RequestIdentity {
    readonly correlationId: string;
    readonly idempotencyKey: string;
    readonly baseRevision: bigint;
    readonly semanticDigest: string;
    readonly promotionNonce: string | null;
  }

  export interface RetryPolicy {
    readonly maximumAttempts: number;
    readonly baseDelayMs: number;
    readonly maximumDelayMs: number;
    readonly jitterRatio: number;
    readonly retryableStatuses: readonly number[];
  }
  ```

  Correlation is browser bookkeeping only. Idempotency is generated once per semantic intent, retained across only compatible retry, and recomputed if operations, proposals, authority envelope, child parameters, or semantic extensions change.
- [ ] Build canonical v1/v2 envelopes through the existing `canonicalize` and `validateUpdateRequest` boundary before fetch. The seed form inserts its intent-owned nonce; instanced and child lifecycle forms cannot carry it.
- [ ] Reserve the semantic request extension `x_suprnova_live_document_key_v1` in both v1 and v2. It carries the bounded document-local root key already emitted by the server and is covered by the existing semantic idempotency digest. Rust parses it into a typed non-authoritative browser render context before action execution; it may be echoed into a successor root but can never select component, instance, scope, route, authorization, or ledger authority. Missing, malformed, changed-on-retry, or cross-island use fails before a response root is published.
- [ ] Generate correlation and idempotency identities from at least 128 bits through the same injected Web Crypto-backed randomness port. Failure closes the server intent; production never falls back to `Math.random`, timestamps, counters, or DOM identity.
- [ ] Fetch only the validated configured endpoint with configured credentials, exact Live request/accept media types, no-store intent, timeout signal, and bounded body read before JSON parse. Classify HTTP/media/network/offline/abort/timeout distinctly and redact all raw response data from diagnostics.
- [ ] Retry only fixture-declared safe interruptions using the same immutable request bytes and identity, injected clock, bounded exponential delay, deterministic injectable jitter, and online signal. User cancellation, island retirement, navigation, incompatible response, and exhausted attempts stop future application without claiming server cancellation.
- [ ] Run protocol fixtures, focused unit tests, deterministic network-order Playwright cases, and Rust v1/v2 golden tests.
- [ ] Commit: `feat(browser): transport Live protocol intents`.

## Task 14: Expose truthful scoped feedback

**Files:**

- Create: `browser/src/feedback/state.ts`
- Create: `browser/src/feedback/targets.ts`
- Create: `browser/src/feedback/timing.ts`
- Create: `browser/src/feedback/announcer.ts`
- Create: `browser/tests/feedback-state.test.ts`
- Create: `browser/tests/feedback-timing.test.ts`
- Create: `browser/e2e/feedback.spec.ts`
- Modify: `browser/src/scheduler/scheduler.ts`
- Modify: `browser/src/models/state.ts`

- [ ] Add failing state-machine cases for idle, dirty, queued, loading, validating, success, interrupted, offline, retrying, error, and combinations across one field, one action, and aggregate island scope. Add delay/minimum-duration/reset, keyboard, busy, disabled, removal, retry, validation versus transport error, and repeated live-region announcement cases.
- [ ] Run focused tests and record failure because scheduler state is not presented.
- [ ] Define feedback as a projection of authoritative scheduler/model state:

  ```ts
  export type FeedbackState =
    | "idle"
    | "dirty"
    | "queued"
    | "loading"
    | "validating"
    | "success"
    | "interrupted"
    | "offline"
    | "retrying"
    | "error";

  export interface FeedbackSnapshot {
    readonly states: ReadonlySet<FeedbackState>;
    readonly intentId: string | null;
    readonly field: string | null;
    readonly action: string | null;
  }
  ```

  Do not let DOM classes or attributes become the source of truth.
- [ ] Resolve generated feedback directives to bounded show/class/attribute/text/live-region targets. Disabled and `aria-busy` state must reflect actual blocking policy and preserve keyboard escape/navigation. A projected optimistic change remains pending, never success.
- [ ] Implement delay and minimum display duration with the injected clock. A delayed state canceled before visibility never flashes; a visible state honors its minimum without postponing scheduler authority or navigation.
- [ ] Coalesce equivalent live-region messages per scope and state transition. Announce validation, interruption, retry, and final failure distinctly; do not announce every render or duplicate response.
- [ ] Run focused unit/browser tests with axe-core and explicit keyboard assertions.
- [ ] Commit: `feat(browser): expose truthful Live feedback`.

## Task 15: Validate and apply responses through one commit-after-morph machine

**Files:**

- Modify: `browser/src/protocol.ts`
- Create: `browser/src/protocol/snapshot-view.ts`
- Create: `browser/src/application/types.ts`
- Create: `browser/src/application/eligibility.ts`
- Create: `browser/src/application/machine.ts`
- Create: `browser/src/application/children.ts`
- Create: `browser/src/application/url.ts`
- Create: `browser/src/application/emissions.ts`
- Create: `browser/tests/response-eligibility.test.ts`
- Create: `browser/tests/application-machine.test.ts`
- Create: `browser/e2e/response-order.spec.ts`
- Modify: `browser/src/scheduler/scheduler.ts`
- Modify: `browser/src/ordering.ts`
- Modify: `src/execution/service.rs`
- Modify: `src/view/root.rs`
- Modify: `tests/endpoint_contract.rs`
- Create: `tests/successor_island_render.rs`

- [ ] Add failing fixture/table cases for wrong media, size, JSON shape, version, correlation, island, base/successor revision, snapshot form, render root, recovery instruction, child envelope, URL target, event/effect schema, extension, duplicate/stale/out-of-order/canceled/superseded/navigation-retired response, and every v3 application trace.
- [ ] Add browser cases proving redirect and v2 navigated URL intent call ordinary navigation and perform no morph, snapshot commit, child queue, URL reflection, event, effect, or in-page success first.
- [ ] Run focused tests and record failure because the current protocol validator returns no typed response and no application machine exists.
- [ ] Refactor protocol parsing to return an immutable validated union while retaining `validateUpdateResponse` as a compatibility wrapper:

  ```ts
  export type ValidatedResponse =
    | ValidatedTerminalNavigation
    | ValidatedCommittedResponse
    | ValidatedRejectedResponse
    | ValidatedRecoveryResponse
    | ValidatedFatalResponse;

  export interface ValidatedCommittedResponse {
    readonly kind: "committed";
    readonly protocol: 1 | 2;
    readonly correlationId: string;
    readonly outcome: "accepted" | "duplicate";
    readonly acceptedRevision: bigint;
    readonly snapshot: Readonly<Record<string, unknown>>;
    readonly render: Readonly<{ kind: "html"; html: string }> | Readonly<{ kind: "no_render" }>;
    readonly validation: Readonly<Record<string, JsonValue>>;
    readonly children: readonly ValidatedChildDelivery[];
    readonly reflectedUrl: string | null;
    readonly events: readonly ValidatedEmission[];
    readonly effects: readonly ValidatedEmission[];
  }
  ```

- [ ] Eligibility combines typed response data with scheduler truth: exact correlation, owning island, expected base revision, legal successor, compatible snapshot form, current connection epoch, application slot, and retirement state. It returns one closed disposition and never mutates DOM or accepted state.
- [ ] Before host commit and ledger acceptance, have the Rust execution path assemble every accepted HTML render through the shared engine-owned root assembler using the newly signed successor snapshot/revision, authoritative instance/component/slot, and the validated inert document key from `x_suprnova_live_document_key_v1`. Public seed promotion produces an instanced successor root. An invalid or oversized wrapper aborts before a successful server outcome exists; no-render remains wrapper-free.
- [ ] Execute terminal navigation immediately after full validation. For committed responses, execute the shared plan exactly: preflight; morph or no-render validation; commit snapshot/revision; reconcile models/validation/focus; queue signed children then same-route reflection; dispatch events; run effects; settle feedback.
- [ ] Child delivery validates target instance/hash/envelope against a surviving child record, then queues one `params_changed` v2 intent through that child's scheduler. Parent completion does not await child acceptance and does not roll back if the child later fails.
- [ ] URL reflection accepts only a same-origin, same-path target whose change is query/fragment policy allowed by fixtures, then calls `history.replaceState` after parent commit. It creates no entry and installs no `popstate` action.
- [ ] Capture the previous browser snapshot/revision before application. Commit only after morph/no-render succeeds. Any failure before commit preserves the old pair; any unexpected failure after commit is classified and recovered without retrying the original action.
- [ ] At commit, decode the bounded non-authoritative successor snapshot view and update the island's instance identity after seed promotion. Require snapshot component/slot/instance/revision to agree with the response and replacement root. Never expose decoded state as authorization or let it override the separately validated accepted revision.
- [ ] Run Rust/TypeScript ordering fixtures, focused unit tests, and all-engine response-order browser tests.
- [ ] Commit: `feat(browser): apply responses after successful morph`.

## Task 16: Put Idiomorph behind Live-owned matching-root preflight

**Files:**

- Create: `browser/src/morph/types.ts`
- Create: `browser/src/morph/limits.ts`
- Create: `browser/src/morph/html.ts`
- Create: `browser/src/morph/keys.ts`
- Create: `browser/src/morph/preflight.ts`
- Create: `browser/src/morph/idiomorph.ts`
- Create: `browser/src/vendor/idiomorph.d.ts`
- Create: `browser/tests/morph-preflight.test.ts`
- Create: `browser/tests/morph-adapter.test.ts`
- Create: `browser/e2e/morph-identity.spec.ts`
- Modify: `browser/src/application/machine.ts`
- Modify: `browser/tests/build-contract.test.ts`

- [ ] Add failing cases for empty/multiple roots, wrong component/slot/document key/instance/successor context, scripts and executable nodes, cross-document nodes, nested island mutation, node/depth/attribute/byte/key/hook/time limits, duplicate/ambiguous keys, keyed reorder, changed key, and adapter failure.
- [ ] Run focused tests and record failure before the adapter exists.
- [ ] Parse accepted HTML through `DOMParser` in an inert document, reject parser errors and prohibited structures, and require exactly one replacement element matching the current island's engine-owned root metadata. Do not insert parsed scripts or import nodes before full preflight.
- [ ] Define the private adapter boundary:

  ```ts
  export interface MorphPlan {
    readonly currentRoot: HTMLElement;
    readonly replacementRoot: HTMLElement;
    readonly identity: IdentityPlan;
    readonly limits: MorphLimits;
  }

  export interface MorphResult {
    readonly root: HTMLElement;
    readonly moved: readonly string[];
    readonly inserted: readonly string[];
    readonly removed: readonly string[];
  }

  export interface MorphAdapter {
    apply(plan: MorphPlan, hooks: MorphHooks): MorphResult;
  }
  ```

  Only `browser/src/morph/idiomorph.ts` imports `idiomorph`. Wrap `Idiomorph.morph` with outer-root mode and Live-owned callbacks; do not export its config, node matching, or callback types.
- [ ] Enforce Live keys before Idiomorph: bounded syntax, uniqueness within ownership scope, stable-key moves preserve logical identity, key change creates identity, and nested islands are opaque keyed component boundaries. Ambiguity fails before DOM mutation rather than delegating a guess to the library.
- [ ] Mark only preflight-approved new nodes as runtime provenance. Prevent incidental script execution and event-attribute activation. Hook budgets and deadline checks cannot be disabled by adapter options.
- [ ] Assert the bundled source contains Idiomorph 0.7.4 provenance/license and no external runtime import. Conformance tests describe resulting Live identity, not exact third-party callback order.
- [ ] Run focused tests, browser identity cases, build/budget checks, and typecheck.
- [ ] Commit: `feat(browser): reconcile islands through private Idiomorph adapter`.

## Task 17: Implement preserve, ignore, replace, persist, and teleport identity

**Files:**

- Create: `browser/src/morph/controls.ts`
- Create: `browser/src/morph/preserve.ts`
- Create: `browser/src/morph/teleport.ts`
- Create: `browser/src/morph/lifecycle.ts`
- Create: `browser/tests/morph-controls.test.ts`
- Create: `browser/tests/teleport.test.ts`
- Create: `browser/e2e/preservation.spec.ts`
- Create: `tests/fixtures/checker/fail/invalid-morph-controls.html`
- Modify: `src/checker/directive.rs`
- Modify: `browser/src/morph/preflight.ts`

- [ ] Add failing checker/unit/browser cases for each control, nested combinations, missing/duplicate keys, incompatible directives, target absence/duplication, cross-island/document teleport, focus/ARIA relationships, third-party subtree authority attempts, repeated morph, removal, and forced replacement disposal.
- [ ] Run focused Rust, Vitest, and Playwright tests; record failure because controls are not part of preflight/lifecycle.
- [ ] Lock distinct semantics:

  ```ts
  export type MorphControl =
    | Readonly<{ kind: "preserve"; key: string }>
    | Readonly<{ kind: "ignore"; key: string; attributes: "server" | "browser" }>
    | Readonly<{ kind: "replace"; key: string }>
    | Readonly<{ kind: "persist"; key: string; destination: string }>
    | Readonly<{ kind: "teleport"; key: string; target: string }>;
  ```

  `preserve` keeps the compatible existing node; `ignore` keeps its descendants under an explicit attribute policy; `replace` forces new identity; `persist` may move within one compatible ownership root; `teleport` uses one checked document-local target and records origin/target lifecycle.
- [ ] Have the Rust checker reject unstable keys, unsafe selectors, cross-owner intent, and incompatible combinations. Browser preflight repeats bounds and ownership checks for defense in depth and rejects dynamic/unproved structure.
- [ ] Capture controlled nodes before mutation, pass only legal preservation decisions into the adapter, reconnect surviving destinations afterward, and dispose removed/replaced controllers, signals, observers, extension resources, and later upload ownership exactly once.
- [ ] Ensure a third-party subtree cannot manufacture an island root, directive provenance, effect registration, action intent, or teleport destination by adding attributes after connection.
- [ ] Run focused checker/unit/all-engine browser tests and morph fixture parity.
- [ ] Commit: `feat(browser): add explicit morph preservation controls`.

## Task 18: Preserve focus, forms, selection, IME, scroll, signals, and controllers

**Files:**

- Create: `browser/src/continuity/types.ts`
- Create: `browser/src/continuity/capture.ts`
- Create: `browser/src/continuity/forms.ts`
- Create: `browser/src/continuity/focus.ts`
- Create: `browser/src/continuity/scroll.ts`
- Create: `browser/src/continuity/restore.ts`
- Create: `browser/tests/continuity.test.ts`
- Create: `browser/e2e/focus-and-forms.spec.ts`
- Create: `browser/e2e/ime-and-selection.spec.ts`
- Create: `browser/e2e/signals-and-controllers.spec.ts`
- Modify: `browser/src/morph/lifecycle.ts`
- Modify: `browser/src/signals/lifecycle.ts`
- Modify: `browser/src/stimulus/lifecycle.ts`

- [ ] Add failing real-browser cases for focused keyed reorder, focus-visible, removed focus with declared/default fallback, text selection/range, contenteditable range, IME composition, scoped scroll, dirty text/check/radio/select/multi-select, file-input ownership, deliberate server correction, signal preserve/rekey/reset/remove, and Stimulus preserve/insert/remove/repeat.
- [ ] Run focused Playwright tests on Chromium, Firefox, and WebKit; record the concrete losses with the raw morph.
- [ ] Capture only bounded state keyed by proven Live identity:

  ```ts
  export interface ContinuityRecord {
    readonly focusedKey: string | null;
    readonly focusVisible: boolean;
    readonly selections: readonly SelectionRecord[];
    readonly composition: CompositionRecord | null;
    readonly controls: readonly ControlContinuity[];
    readonly scroll: readonly ScrollContinuity[];
    readonly signalScopes: readonly SignalContinuity[];
    readonly stimulus: StimulusContinuity;
  }
  ```

  Cap each collection and total retained bytes. Never retain arbitrary subtree HTML, file bytes, snapshot contents, or application objects.
- [ ] Capture after response preflight but before mutation. The adapter may preserve surviving node identity, but it does not restore captured browser state before authority commit. After the morph succeeds, the application machine commits snapshot/revision and then performs model, validation, selection, composition, scroll, signal, controller, and focus reconciliation in the shared semantic order. Any reconciliation failure follows Task 19's application-order recovery with the previous browser snapshot retained.
- [ ] A newer browser edit wins presentation over an older accepted proposal while accepted server state advances internally. Explicit authoritative correction metadata wins only for its declared field and sequence. File nodes are preserved/moved as owned browser objects or the morph is rejected; file values are never synthesized.
- [ ] Keep signal values only when island, keyed scope, declaration name/type, and preserve policy match. Reset/rekey/replacement/removal disposes once. Coordinate Stimulus through Task 10's public bridge and DOM identity, not Idiomorph internals.
- [ ] Restore focus to the surviving identity, then an application-declared safe target, then a semantic island fallback. Respect fragment/navigation focus separately and never focus hidden/inert/disabled content.
- [ ] Run all continuity suites, axe-core, keyboard assertions, and leak checks for repeated morph cycles.
- [ ] Commit: `feat(browser): preserve interaction continuity across morphs`.

## Task 19: Add bounded transitions and fresh-render recovery

**Files:**

- Create: `browser/src/transitions/types.ts`
- Create: `browser/src/transitions/runner.ts`
- Create: `browser/src/transitions/lifecycle.ts`
- Create: `browser/src/application/recovery.ts`
- Create: `browser/tests/transitions.test.ts`
- Create: `browser/tests/recovery.test.ts`
- Create: `browser/e2e/transitions-and-recovery.spec.ts`
- Modify: `browser/src/application/machine.ts`
- Modify: `browser/src/feedback/state.ts`

- [ ] Add failing cases for enter/leave/move/state transition, cancellation, supersession, timeout, animation rejection, reduced motion, missing API, removal, navigation, stale feedback, first morph failure, second recovery failure, and late original response.
- [ ] Run focused tests and record failure before transition/recovery state exists.
- [ ] Define transition execution as bounded presentation around an authority operation:

  ```ts
  export interface TransitionSpec {
    readonly kind: "enter" | "leave" | "move" | "state";
    readonly name: string;
    readonly maximumMs: number;
    readonly essential: boolean;
  }

  export type RecoveryState = "none" | "fresh_render_pending" | "disconnected";
  ```

  Use checked names and Web Animations/CSS completion ports. Reduced motion completes nonessential motion immediately. Timeout/cancel applies the semantic final state and cannot hold loading/disabled state or accepted authority indefinitely.
- [ ] On morph or application-order failure after server acceptance, keep the previous browser snapshot/revision, invalidate original response application, and enqueue exactly one v2 `fresh_render` intent without model proposals, child parameters, or original action. Never retry original request bytes.
- [ ] Cap recovery attempts per accepted revision/connection epoch. A second failed recovery disconnects only that island, aborts its future application, exposes last accepted/SSR HTML, and emits one redacted diagnostic. It does not reload the document automatically unless the validated server recovery instruction says ordinary navigation.
- [ ] Ensure projections roll back, feedback becomes interrupted/recovering rather than success, late animation/effect/response callbacks are ignored by epoch, and transition failure cannot veto security validation.
- [ ] Run focused unit/all-engine browser cases with injected completion rather than time sleeps.
- [ ] Commit: `feat(browser): bound transitions and morph recovery`.

## Task 20: Enhance real navigation without installing a client router

**Files:**

- Create: `browser/src/navigation/eligibility.ts`
- Create: `browser/src/navigation/native.ts`
- Create: `browser/src/navigation/prefetch.ts`
- Create: `browser/src/navigation/view-transitions.ts`
- Create: `browser/src/navigation/guards.ts`
- Create: `browser/src/navigation/focus-scroll.ts`
- Create: `browser/tests/navigation-eligibility.test.ts`
- Create: `browser/tests/prefetch.test.ts`
- Create: `browser/e2e/navigation.spec.ts`
- Create: `browser/e2e/document-transitions.spec.ts`
- Modify: `browser/src/application/url.ts`
- Modify: `browser/src/runtime/config.ts`

- [ ] Add failing unit/browser cases for ordinary anchors, GET/POST forms, redirects, refresh, fragments, download, external origin, `target`, modifier keys, new tab, content negotiation, error documents, same-route reflection, Back/Forward, dirty guard leave/stay, focus/scroll, reduced motion, unsupported View Transitions, capture failure, and cancellation.
- [ ] Add prefetch failures for non-GET/HEAD, credentials/tenant/principal/locale variance, flash consumption, no-store/private server policy, data saver, cross-origin, redirect-prone, excessive concurrency, hidden target, and cancellation. Assert the runtime never fetches and installs a document body.
- [ ] Run focused tests and record failure before navigation enhancement exists.
- [ ] Define native navigation intent only:

  ```ts
  export interface NativeNavigationIntent {
    readonly target: URL;
    readonly method: "GET" | "HEAD" | "POST";
    readonly history: "navigate" | "replace_query";
    readonly prefetch: "none" | "link" | "speculation";
    readonly transitionName: string | null;
  }
  ```

  Activation ultimately uses the browser's normal anchor/form/location/history behavior and complete documents. There is no document-body fetch port, partial-document response, route table, popstate action, or client navigation state store.
- [ ] Keep same-route reflection limited to the validated response path from Task 15. Use `replaceState`, preserve the current history entry, and ignore `popstate` except for normal lifecycle compatibility checks.
- [ ] For fixture-eligible safe targets, emit bounded native `<link rel="prefetch">` or a CSP-compatible Speculation Rules declaration using checked absolute same-origin URLs and host config. Honor data saver, concurrency, cache/privacy metadata, cancellation, and removal. Never read the prefetched body in JavaScript.
- [ ] Feature-detect cross-document View Transitions and checked unique names. If unsupported, reduced-motion, cross-origin, download, error, timeout, or capture failure applies ordinary navigation with identical method/target/history semantics.
- [ ] Implement an accessible dirty-work guard with application-declared text, focus return, and a guaranteed leave path. It warns about uncommitted browser work but never claims an already committed server action was rolled back.
- [ ] Run all-engine navigation tests, manual agent-browser keyboard/Back/Forward exploration with fresh accessibility snapshots, and format/lint/typecheck.
- [ ] Commit: `feat(browser): enhance native document navigation`.

## Task 21: Handle page lifecycle, bfcache, and exact cleanup

**Files:**

- Create: `browser/src/lifecycle/events.ts`
- Create: `browser/src/lifecycle/document.ts`
- Create: `browser/src/lifecycle/bfcache.ts`
- Create: `browser/src/lifecycle/resources.ts`
- Create: `browser/tests/document-lifecycle.test.ts`
- Create: `browser/e2e/bfcache.spec.ts`
- Create: `browser/e2e/resource-lifecycle.spec.ts`
- Modify: `browser/src/runtime/runtime.ts`
- Modify: `browser/src/bootstrap.ts`

- [ ] Add failing deterministic cases for `pagehide`, persisted/non-persisted hide, `freeze`, `resume`, `pageshow`, unload limitations, late fetch/animation/observer callbacks, real document replacement, bfcache restoration with compatible/incompatible asset/version, duplicate pageshow, and repeated start/suspend/resume/dispose.
- [ ] Run focused unit/browser tests and record duplicated or missing resource behavior.
- [ ] Implement an explicit document state machine:

  ```ts
  export type DocumentRuntimeState =
    | "created"
    | "active"
    | "suspended"
    | "restoring"
    | "disposed";

  export interface ResourceLedger {
    add(kind: ResourceKind, dispose: () => void): Disposable;
    suspend(): void;
    resume(): void;
    dispose(): void;
    counts(): Readonly<Record<ResourceKind, number>>;
  }
  ```

  Every listener, observer, timer, transport, transition, controller bridge, scheduler, signal scope, and extension lease registers once in the ledger and has idempotent disposal.
- [ ] On true replacement, dispose document resources and reset document-scoped signals. On persisted pagehide/freeze, suppress application and suspend allowable work without claiming server cancellation. On pageshow/resume, increment the connection epoch, validate asset/runtime/protocol/island compatibility, reject late responses from the old epoch, and reconnect through the ordinary discovery path.
- [ ] Never register an `unload` handler. Attach `beforeunload` only while an explicit dirty-work guard is active and remove it as soon as the guard clears so clean documents remain bfcache-eligible. Use `pagehide`/`pageshow` as the reliable lifecycle boundary and feature-detect freeze/resume.
- [ ] Duplicate artifact execution after restoration returns the existing compatible runtime and never creates a second observer/listener/controller/scheduler. Incompatibility leaves current HTML exposed and requires ordinary document refresh according to diagnostics/recovery policy.
- [ ] Add test-only resource counts and weak-reference/finalization probes behind a non-production entry. Production diagnostics expose only bounded aggregate closed metrics.
- [ ] Run all-engine bfcache/resource tests and use DevTools MCP to inspect bfcache eligibility and retained listeners/observers. Record diagnosis as exploratory, not release evidence.
- [ ] Commit: `feat(browser): make document lifecycle idempotent`.

## Task 22: Close hostile DOM, accessibility, CSP, and browser conformance gaps

**Files:**

- Create: `browser/e2e/accessibility.spec.ts`
- Create: `browser/e2e/hostile-dom.spec.ts`
- Create: `browser/e2e/compatibility.spec.ts`
- Create: `browser/e2e/leaks.spec.ts`
- Create: `browser/e2e/full-flow.spec.ts`
- Create: `browser/e2e/support/a11y.ts`
- Create: `browser/e2e/support/faults.ts`
- Create: `browser/docs/exploratory-browser-qa.md`
- Modify: `browser/test-host/scenarios.mjs`

- [ ] Build a traceability table in the test source covering nested ownership, multiple schedulers, local-only interaction, seed promotion, event ownership, models/forms, response order, morph identity, focus/selection/IME, controllers/signals, controls/teleport, transitions/effects, redirect/reflection, offline/retry/cancel, navigation/bfcache, CSP, diagnostics, accessibility, and recovery.
- [ ] Add hostile cases for extreme depth/count/attributes/text, duplicate keys/roots/identity, third-party mutations, shadow DOM, returned scripts/event handlers, malformed UTF-8 response bytes, huge JSON, prototype-shaped keys, throwing getters/proxies at public APIs, and callbacks after retirement. Every case has one closed bounded outcome.
- [ ] Add axe-core assertions plus explicit semantic/keyboard checks for disclosures, tabs, form errors, feedback/live regions, disabled/busy/inert, focus recovery, dirty guards, reduced motion, and ordinary fallback. Automated scans do not replace explicit focus/keyboard assertions.
- [ ] Test external module and classic assets under nonce-only and hash-only CSP. Assert production paths contain no inline executable code, `eval`, `new Function`, dynamic module URL, server-returned script execution, or silently enabled verbose diagnostics.
- [ ] Repeat connect/morph/remove/suspend/restore cycles with deterministic clocks and network. Assert resource-ledger counts return to baseline and no retired island accepts a callback. Heap measurements remain Task 24's budget evidence.
- [ ] Document the project-local exploratory workflow: derive a unique agent-browser session, open the built test host, snapshot before interaction, use fresh refs after each DOM change, exercise semantic controls, inspect network/accessibility, capture only redacted screenshots, and close the session. Document DevTools MCP checks for lifecycle, memory, performance, observers, and bfcache.
- [ ] Run the complete Playwright suite on pinned Chromium, Firefox, and WebKit. Never label Playwright WebKit as Safari.
- [ ] Commit: `test(browser): harden real-browser conformance`.

## Task 23: Add provider-neutral actual-browser qualification evidence

**Files:**

- Create: `browser/compatibility/matrix.json`
- Create: `browser/compatibility/schema.json`
- Create: `browser/compatibility/README.md`
- Create: `browser/scripts/run-compatibility.mjs`
- Create: `browser/scripts/check-compatibility.mjs`
- Create: `browser/tests/compatibility-evidence.test.ts`
- Create: `browser/compatibility/results/.gitkeep`
- Modify: `browser/package.json`

- [ ] Add a failing evidence test requiring exact matrix entries for minimum Chrome 111, Edge 111, Firefox 128, Safari 16.4, and each current stable channel; provider, actual product/version, OS, artifact SHA-256, fixture manifest, timestamp, result, and attestation must be explicit.
- [ ] Run the focused test and record failure because the qualification model is absent.
- [ ] Define release evidence independently of Playwright projects:

  ```ts
  export interface CompatibilityEvidence {
    readonly schemaVersion: 1;
    readonly browserProduct: "chrome" | "edge" | "firefox" | "safari";
    readonly browserVersion: string;
    readonly operatingSystem: string;
    readonly provider: string;
    readonly runtimeSha256: string;
    readonly fixtureManifestSha256: string;
    readonly executedAt: string;
    readonly result: "pass" | "fail";
    readonly attestation: string;
  }
  ```

  Reject aliases such as WebKit for Safari, Chromium for Chrome/Edge, user-agent-only claims, missing artifact identity, stale fixture identity, and self-declared simulated products.
- [ ] Make the runner provider-neutral: it serves the same production assets and test catalog, accepts a remote WebDriver/CDP provider adapter outside the core runtime, and writes evidence only after every required conformance case returns an authenticated test-run nonce and artifact hash. Provider credentials never enter output.
- [ ] `compatibility:check` distinguishes `qualified`, `failed`, and `unqualified`. Missing actual-floor evidence returns `unqualified` and blocks release qualification, while ordinary local implementation commands may continue with an explicit unqualified label.
- [ ] Check in the matrix/schema/empty results directory, not fabricated passing evidence. Add instructions for actual Safari/macOS and legacy browser providers without naming any one commercial service as normative.
- [ ] Run schema/evidence tests and verify the normal Playwright gate remains a separate requirement.
- [ ] Commit: `test(browser): define actual-browser qualification matrix`.

## Task 24: Enforce transfer, D100, M1K, M5K, idle, and retained-memory budgets

**Files:**

- Create: `browser/benchmarks/workloads.ts`
- Create: `browser/benchmarks/runner.ts`
- Create: `browser/benchmarks/statistics.ts`
- Create: `browser/benchmarks/schema.ts`
- Create: `browser/benchmarks/baselines/browser-budget-v1.json`
- Create: `browser/benchmarks/local/.gitignore`
- Create: `browser/scripts/run-browser-budget.mjs`
- Modify: `browser/scripts/check-budget.mjs`
- Create: `browser/tests/benchmark-contract.test.ts`
- Create: `browser/e2e/performance.spec.ts`
- Modify: `browser/package.json`

- [ ] Add failing benchmark-contract tests for workload shape, environment identity, warmup, at least 30 measured samples for B1, p50/p95 calculation, artifact hash, browser revision, viewport, four-times CPU throttle, host hardware, observer count, idle network/polling, retained-memory methodology, hard caps, baseline comparison, noise band, and three-run regression confirmation.
- [ ] Run the focused test and current budget script; record failure because only the conformance-package transfer check exists.
- [ ] Generate canonical workloads exactly: `D100` is a 64 KiB document with 100 connected islands; `M1K` is one keyed 1,000-element/depth-12 island with ten percent changed nodes; `M5K` is one keyed 5,000-element/depth-24 island with ten percent changed nodes.
- [ ] Measure production core Brotli bytes including Idiomorph and excluding the
  optional Stimulus package and Suprnova bridge/continuity implementation,
  diagnostics extras, maps, and component CSS. Enforce at most 45 KiB and prove
  the exclusion from core metafiles.
- [ ] On the recorded B1 environment enforce: D100 connect at most 50 ms p95; 30 idle seconds at most 5 ms total main-thread time; exactly one core mutation observer; no polling/network; at most 12 KiB incremental retained runtime memory per island excluding DOM and raw document HTML/snapshot strings; M1K at most 32 ms p95; M5K at most 100 ms p95.
- [ ] Record local runs as `exploratory` unless every B1 field matches. Missing B1 proof cannot pass a release request. A checked baseline regression of 15 percent or more requires three independent confirmations; within five percent is noise. Correctness/accessibility/lifecycle work stays enabled during measurements.
- [ ] Use Chromium tracing/CDP only through the benchmark runner's measurement port. Add DevTools MCP spot checks for observer/heap interpretation, but do not treat them as the JSON baseline.
- [ ] Run deterministic benchmark contract, production bundle budget, an exploratory local D100/M1K/M5K run, and validate the result schema. Check in the reviewed baseline with honest environment classification.
- [ ] Commit: `perf(browser): enforce Live runtime budgets`.

## Task 25: Add parser properties, fuzz regressions, and security boundaries

**Files:**

- Create: `browser/tests/config-properties.test.ts`
- Create: `browser/tests/directive-properties.test.ts`
- Create: `browser/tests/scheduler-properties.test.ts`
- Create: `browser/tests/morph-properties.test.ts`
- Create: `browser/tests/diagnostic-redaction-properties.test.ts`
- Create: `fuzz/fuzz_targets/directive_contract.rs`
- Create: `fuzz/fuzz_targets/browser_metadata.rs`
- Modify: `fuzz/Cargo.toml`
- Modify: `fuzz/fuzz_targets/support.rs`
- Create: `tests/browser_contract_properties.rs`
- Modify: `tests/security_boundaries.rs`

- [ ] Add fast-check generators for nested JSON, hostile directive names/values/modifiers, event sequences, scheduler commands, key forests, lifecycle traces, diagnostic inputs, and DOM metadata byte strings. Assert total functions, bounded output, stable error codes, no prototype mutation, and no secret/raw echo.
- [ ] Add Rust property/fuzz targets for the same shared grammar and engine-emitted browser metadata. Seed corpora from v3 success/failure fixtures and preserve every discovered crash as a checked regression case.
- [ ] Run focused property tests with fixed seeds and bounded case counts; record failures before hardening any exposed parser.
- [ ] Harden only the proven boundaries. Avoid catch-all exception swallowing; normalize expected hostile input to closed results and keep internal programmer defects visible in tests.
- [ ] Run bounded nightly fuzz smoke campaigns for new targets plus all existing Iteration 001/002 targets. Build success alone is not campaign evidence.
- [ ] Prove no production dependency adds unsafe evaluation, blanket warning denial, unbounded DOM traversal, raw secret-bearing formatting, or browser-created authority.
- [ ] Commit: `test: fuzz browser-runtime boundaries`.

## Task 26: Integrate the unattended gate and implementation documentation

**Files:**

- Modify: `scripts/gate.sh`
- Modify: `tests/gate_contract.sh`
- Modify: `scripts/generate-license-inventory.mjs`
- Modify: `THIRD_PARTY_LICENSES.md`
- Create: `docs/implementation/browser-runtime.md`
- Create: `docs/implementation/browser-assets.md`
- Create: `docs/implementation/live-directives.md`
- Create: `docs/implementation/local-reactivity.md`
- Create: `docs/implementation/scheduling-and-feedback.md`
- Create: `docs/implementation/morphing-and-continuity.md`
- Create: `docs/implementation/document-navigation.md`
- Create: `docs/implementation/browser-testing.md`
- Modify: `docs/implementation/fixtures.md`
- Modify: `docs/implementation/benchmarking.md`
- Modify: `docs/implementation/threat-model-v1.md`
- Modify: `scripts/check-implementation-docs.mjs`
- Modify: `tests/documentation_contract.sh`
- Modify: `docs/specs/suprnova-live.zip`

- [ ] Add failing gate/doc assertions for exact lockfile install; generator drift; deterministic assets; format/lint/typecheck; Vitest; shared fixtures; Playwright Chromium/Firefox/WebKit; CSP; accessibility; leaks; compatibility qualification labeling; bundle and browser budgets; Rust checker/conformance; MSRV; fuzz; specs/archive; and licenses.
- [ ] Run the focused shell contracts and record every missing phase.
- [ ] Update the gate in dependency order. Use `rtk npm ci`, never a floating install. Run Playwright's pinned projects after production `build:check`; run actual-browser `compatibility:check` in release-aware mode; do not mislabel missing actual floors as a local test failure or a release pass.
- [ ] Keep Rust format/Clippy/tests/MSRV/fuzz and Iteration 001/002 budgets intact. Review warnings directly; gate-contract tests reject `-D warnings` and warning-suppression shortcuts.
- [ ] Generate the license inventory for Idiomorph and all new build/test dependencies, preserving required notices in production distribution metadata. Test-only packages remain identified as test-only.
- [ ] Document artifact serving/configuration, closed directives, local versus server action flow, models/scheduling/feedback, effects/public calls, optional Stimulus, identity/morph controls, focus/forms/IME, recovery, native navigation/prefetch/View Transitions, lifecycle/bfcache, CSP, actual browser support, diagnostic tools, fixtures, and budget reproduction. Examples use final `suprnova::live` concepts while clearly labeling this host and package as standalone development machinery.
- [ ] Regenerate the optional Fable archive exactly:

  ```bash
  rtk proxy bash -lc 'cd docs/specs && zip -X -q -FS -r suprnova-live.zip suprnova-live -i "*.md" -x "suprnova-live/iterations/next/*"'
  rtk node scripts/check-specs.mjs
  ```

- [ ] Run doc contracts, link checks, license check, archive parity, and `rtk git diff --check`.
- [ ] Commit: `docs: record Live browser runtime`.

## Task 27: Run the complete Iteration 003 gate and final self-audit

**Files:**

- Review: every tracked Iteration 003 file
- Modify: only defects proven by checks or final audit

- [ ] Run `rtk node scripts/check-specs.mjs` and `rtk git diff --check`.
- [ ] Run deterministic browser contract generation in check mode.
- [ ] Run `rtk npm --prefix browser ci`, then format check, lint, typecheck, unit tests, build, build check, bundle budget, Playwright Chromium/Firefox/WebKit, compatibility evidence check, and browser benchmark contract.
- [ ] Run an exploratory D100/M1K/M5K measurement and validate its honest non-B1 or B1 classification. Do not infer release qualification from ordinary Playwright results.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo fmt --all --check`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features` and review every warning without blanket denial.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --workspace --all-targets --all-features --no-fail-fast` and `rtk env CARGO_INCREMENTAL=0 cargo test --workspace --doc --all-features`.
- [ ] Run checker positive/negative/regression/property suites, Rust/TypeScript v1-v3 fixtures, and macro UI tests explicitly.
- [ ] Run the pinned Rust 1.91.1/MSRV checks, all expansion/action/snapshot budgets, nightly fuzz build, and bounded smoke campaigns for every target.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 scripts/gate.sh`.
- [ ] Use agent-browser against the built host for one complete keyboard/local-action/server-action/morph/navigation path, refreshing refs after DOM changes and closing the session. Use DevTools MCP for one observer/heap/bfcache diagnostic pass. Record both as exploratory corroboration only.
- [ ] Inspect the complete diff, tracked inventory, dependency tree, generated asset metadata, and production bundle. Search for placeholder markers, unimplemented agreed branches, stale Iteration 002 labels, unsafe evaluation, raw HTML/script execution, secret-bearing diagnostics, unbounded external input, duplicate runtime resources, arbitrary endpoints/modules, upload/stream/cache/component-library work, or false SPA/Suprnova-integration claims.
- [ ] Map fresh evidence to all 31 completion conditions below. Re-run every check touched by remediation; stale output is not evidence.
- [ ] Inspect only the status and current commit of active Suprnova and Magnetar worktrees. Confirm no Iteration 003 command wrote to either repository.
- [ ] Commit the verified final state locally without pushing: `feat: complete Suprnova Live iteration 003`.

## Definition-of-done coverage matrix

| Iteration 003 condition | Primary tasks | Required evidence |
| --- | --- | --- |
| 1. Reproducible ESM/classic artifacts and manifest | 1, 5, 27 | package/build contract, byte-identical rebuild, manifest hashes |
| 2. Bounded config, duplicate startup, safe failure | 4-6, 21 | config/diagnostic tests, CSP/duplicate-load Playwright |
| 3. One observer/listener set and deterministic island lifecycle | 6, 7, 21 | discovery, dynamic insertion, resource-ledger tests |
| 4. Seed no-eager-request and first-intent nonce | 6, 7, 13 | nonce properties and seed browser flow |
| 5. Nested ownership, exact retirement, lazy completion | 6, 7, 21 | nested/lazy/removal traces and leak checks |
| 6. Rust/browser closed grammar agreement | 2, 3, 25 | generated parity, checker fixtures, parser properties |
| 7. Delegated event semantics | 7, 22 | event-routing properties and real-browser ownership cases |
| 8. Typed local signals and accessible presentation | 8, 22 | signal/presentation unit, hostile, keyboard, axe evidence |
| 9. Signal morph continuity and disposal | 8, 18, 21 | identity lifecycle and repeated-morph leak tests |
| 10. Optional Stimulus without core bundle dependency | 5, 10, 18 | actual Stimulus lifecycle cases and bundle inspection |
| 11. Effects, public calls, optimistic projection | 9, 15, 19 | schema/ownership/failure/projection tests |
| 12. One bounded scheduler per island | 7, 11, 21 | command-model properties and multi-island browser cases |
| 13. Model timing and newer-edit protection | 12, 15, 18 | injected-clock model/form and reconciliation tests |
| 14. Compatible bounded transport | 7, 13, 25 | v1/v2 request fixtures and hostile network cases |
| 15. Truthful feedback | 14, 19, 22 | state/timing/live-region/keyboard evidence |
| 16. Response eligibility and terminal navigation | 2, 15 | shared ordering fixtures and terminal browser trace |
| 17. Exact nonterminal order, children, URL reflection | 2, 15 | Rust/TS trace parity and child scheduler tests |
| 18. Distinct response dispositions and bounded recovery | 11, 15, 19 | state-machine/fault/recovery-loop tests |
| 19. Live-owned morph preflight and private Idiomorph | 5, 16, 25 | preflight/adapter/hostile DOM/bundle evidence |
| 20. Keys and morph controls | 3, 16, 17 | checker, preflight, reorder/control/teleport cases |
| 21. Browser interaction continuity | 12, 18, 22 | all-engine focus/forms/selection/IME/a11y cases |
| 22. Bounded deterministic transitions | 19, 20 | transition state-machine and fallback cases |
| 23. Native navigation and enhancements | 15, 20, 22 | native semantics, replaceState, prefetch, transition cases |
| 24. Page lifecycle, bfcache, and leaks | 21, 22, 24 | lifecycle traces, bfcache, resource and heap budgets |
| 25. Complete browser/parser/security suites | 2-25 | unit/property/fixture/checker/Playwright/CSP/a11y/fuzz gate |
| 26. Pinned engines and actual-floor qualification | 22, 23, 27 | Playwright gate plus provider-neutral evidence classification |
| 27. D100/M1K/M5K recorded workloads | 24, 27 | schema-valid exploratory or B1 result with p50/p95 |
| 28. Hard caps and regression gate | 5, 24, 27 | Brotli, bootstrap, idle, observer, memory, morph checks |
| 29. Complete unattended gate without warning denial | 1, 23-27 | gate contract and successful complete run |
| 30. Complete implementation documentation | 20, 21, 23, 24, 26 | doc contract, examples, testing and budget reproduction |
| 31. No drift, crossing, placeholders, or push | every task, 26, 27 | spec/archive parity, inventory audit, read-only external status |

## Plan self-review checklist

- [ ] Every Iteration 003 definition-of-done condition maps to at least one implementation task and fresh verification artifact.
- [ ] Every new production parser/state machine has positive, negative, bounded hostile, and disposal coverage.
- [ ] Rust checker, TypeScript runtime, fixtures, response order, and generated artifacts have one stated source of truth and drift check.
- [ ] ESM/classic, Stimulus-free core including bridge/lifecycle source
  exclusion, optional adapter parity, Idiomorph provenance, CSP, actual-browser
  naming, B1 qualification, and the no-SPA boundary are mechanically checked.
- [ ] No task modifies or depends on active Suprnova/Magnetar paths, adds upload/stream/cache/component-library scope, uses blanket warning denial, relies on sleep for correctness, or authorizes a push.
- [ ] Search this plan for unresolved placeholders, inconsistent type names, missing files, non-`rtk` commands, and assertions that lack an executable check before beginning implementation.
