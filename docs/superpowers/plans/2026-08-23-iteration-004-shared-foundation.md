# Iteration 004 Shared Foundation and Optional Artifacts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the version-4 cross-language contracts, shared bounded-resource primitives, optional-feature lifecycle seam, and deterministic Stimulus/upload/async browser artifacts required by Iteration 004 without changing Live action/morph protocol v2.

**Architecture:** Add small host-neutral bounded queues, permits, cancellation, ownership, and diagnostics in Rust and TypeScript. The core browser runtime owns one closed lifecycle-driver attachment; one optional driver carries the existing Stimulus adapter singleton plus the fixed upload and async feature slots but cannot install HTML, invoke arbitrary actions, or mint authority. Island discovery exposes narrow connect/retire and morph-lifecycle hooks rather than absorbing optional implementations. ESM applications load typed adapter registrations before `boot`; classic artifacts register through one versioned global symbol. The manifest identifies roles and compatibility, while directive attributes never select executable URLs.

**Tech Stack:** Rust 1.91.1, strict TypeScript 6.0.3, serde 1.0.229, Vitest 4.1.11, fast-check 4.9.0, esbuild 0.28.2, terser 5.50.0, native DOM lifecycle APIs, existing deterministic fixture and manifest tooling.

---

## Dependencies and execution rules

- This is Plan 1 of 4 and is the prerequisite for the upload and asynchronous-update plans.
- Work only in `/home/shawn/workspace2/suprnova-live/.worktrees/iteration-004-uploads-async` on branch `iteration-004-uploads-async`.
- Start every shell command with `rtk`; use `rtk proxy` for a raw subordinate command.
- Use `apply_patch` for hand edits. Formatting and deterministic generators may rewrite only their owned outputs.
- Follow red/green/refactor: add the smallest failing test, run it and record the expected failure, implement the smallest complete behavior, rerun the focused and neighboring suites, then commit.
- Use injected clocks, schedulers, randomness, connectivity, transport, and lifecycle events. Correctness tests must not depend on elapsed sleeps.
- Do not use blanket `-D warnings`.
- Do not modify `/home/shawn/workspace2/suprnova` or `/home/shawn/workspace2/suprnova-magnetar`.
- Make each task's commit locally. Never push this branch unless the developer explicitly authorizes it.

## File structure

### Create

- `fixtures/v4/directive-grammar.json`
- `fixtures/v4/runtime-features.json`
- `fixtures/v4/upload-protocol.json`
- `fixtures/v4/async-envelope.json`
- `fixtures/v4/resource-lifecycle.json`
- `fixtures/v4/diagnostics.json`
- `fixtures/v4/compatibility.json`
- `fixtures/v4/manifest.sha256`
- `src/resource/mod.rs`
- `src/resource/bounds.rs`
- `src/resource/cancel.rs`
- `src/resource/owner.rs`
- `src/resource/queue.rs`
- `browser/src/features/contract.ts`
- `browser/src/features/host.ts`
- `browser/src/features/global.ts`
- `browser/src/features/bounded.ts`
- `browser/src/entry-uploads-esm.ts`
- `browser/src/entry-uploads-classic.ts`
- `browser/src/entry-async-esm.ts`
- `browser/src/entry-async-classic.ts`
- `browser/tests/feature-host.test.ts`
- `browser/tests/bounded-resources.test.ts`
- `browser/tests/optional-artifacts.test.ts`
- `tests/resource_foundation.rs`

### Modify

- `src/lib.rs`
- `src/conformance.rs`
- `scripts/generate-browser-contracts.mjs`
- `src/checker/generated_directive_contract.rs`
- `browser/src/generated/directive-contract.ts`
- `browser/src/conformance.ts`
- `browser/src/assets.ts`
- `browser/src/runtime/types.ts`
- `browser/src/runtime/runtime.ts`
- `browser/src/islands/discovery.ts`
- `browser/src/islands/record.ts`
- `browser/src/lifecycle/resources.ts`
- `browser/src/entry-esm.ts`
- `browser/src/entry-classic.ts`
- `browser/scripts/build.mjs`
- `browser/scripts/check-build.mjs`
- `browser/scripts/check-budget.mjs`
- `browser/package.json`
- `browser/tests/golden-fixtures.test.ts`
- `tests/golden_fixtures.rs`
- `tests/browser_contract_properties.rs`
- `scripts/gate.sh`

## Task 1: Add the version-4 fixture catalog without changing protocol v2

**Files:** `fixtures/v4/*`, `src/conformance.rs`, `browser/src/conformance.ts`, `tests/golden_fixtures.rs`, `browser/tests/golden-fixtures.test.ts`

- [ ] Add failing Rust and TypeScript catalog tests that require the exact v4 file set and prove v1/v2 wire fixtures remain unchanged:

  ```rust
  #[test]
  fn version_four_is_an_independent_capability_fixture_set() {
      assert_eq!(fixture_files_v4(), &[
          "async-envelope.json",
          "compatibility.json",
          "diagnostics.json",
          "directive-grammar.json",
          "resource-lifecycle.json",
          "runtime-features.json",
          "upload-protocol.json",
      ]);
      assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);
  }
  ```

  ```ts
  expect(FIXTURE_FILES_V4).toEqual([
    "async-envelope.json",
    "compatibility.json",
    "diagnostics.json",
    "directive-grammar.json",
    "resource-lifecycle.json",
    "runtime-features.json",
    "upload-protocol.json",
  ]);
  expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1, 2]);
  ```

- [ ] Run `rtk cargo test --test golden_fixtures version_four` and `rtk npm --prefix browser test -- golden-fixtures.test.ts`; record failure because v4 is absent.
- [ ] Add canonical bounded cases for feature compatibility, ownership retirement, upload states/codecs, async sequences/continuity, diagnostics redaction, and the four directives. Export the exact list in both languages:

  ```rust
  pub const FIXTURE_FILES_V4: &[&str] = &[
      "async-envelope.json",
      "compatibility.json",
      "diagnostics.json",
      "directive-grammar.json",
      "resource-lifecycle.json",
      "runtime-features.json",
      "upload-protocol.json",
  ];
  ```

  ```ts
  export const FIXTURE_FILES_V4 = Object.freeze([
    "async-envelope.json",
    "compatibility.json",
    "diagnostics.json",
    "directive-grammar.json",
    "resource-lifecycle.json",
    "runtime-features.json",
    "upload-protocol.json",
  ] as const);
  ```

- [ ] Generate `fixtures/v4/manifest.sha256` with the existing canonical filename/content algorithm, run both focused suites plus all golden tests, and verify no v1/v2 file changed with `rtk git diff -- fixtures/v1 fixtures/v2`.
- [ ] Commit: `test: add iteration 004 capability fixtures`.

## Task 2: Generate the closed v4 directive and role contract

**Files:** `fixtures/v4/directive-grammar.json`, `scripts/generate-browser-contracts.mjs`, generated Rust/TypeScript contracts, checker/property tests

- [ ] Add failing checker and parser tests for `live:upload`, `live:progress`, `live:poll`, and `live:stream`, including role/modifier conflicts and inert fallback:

  ```rust
  #[test]
  fn iteration_four_directives_have_closed_capability_contracts() {
      let upload = directive_contract("upload").expect("upload contract");
      assert_eq!(upload.capability, Some("uploads@1"));
      assert!(upload.roles.contains(&"cancel"));
      assert_eq!(directive_contract("stream").unwrap().capability, Some("async@1"));
  }
  ```

  ```ts
  expect(
    parseDirective("live:poll.visible.30s", "refresh").diagnostic,
  ).toBeNull();
  expect(parseDirective("live:upload.stream", "avatar").diagnostic?.code).toBe(
    "unsupported_modifier",
  );
  ```

- [ ] Run `rtk cargo test --test checker_positive --test checker_negative` and `rtk npm --prefix browser test -- directive-parser.test.ts`; record failure on unknown directives/schema fields.
- [ ] Upgrade the generator to read `fixtures/v4`, accept schema version 2, and emit explicit capability and role fields:

  ```rust
  pub struct DirectiveContract {
      pub name: &'static str,
      pub owner: DirectiveOwner,
      pub value: DirectiveValue,
      pub modifiers: &'static [&'static str],
      pub roles: &'static [&'static str],
      pub conflicts: &'static [&'static str],
      pub phase: DirectivePhase,
      pub fallback: DirectiveFallback,
      pub capability: Option<&'static str>,
  }
  ```

  ```ts
  export type RuntimeDirectiveContract = readonly [
    name: string,
    value: 0 | 1 | 2 | 3 | 4 | 5 | 6,
    modifiers: readonly string[],
    roles: readonly string[],
    conflicts: readonly string[],
    fallback: 0 | 1 | 2,
    capability: "uploads@1" | "async@1" | null,
  ];
  ```

- [ ] Run `rtk npm --prefix browser run generate`, then `generate:check`, checker suites, parser properties, format, and `rtk git diff --check`.
- [ ] Commit: `feat(checker): generate iteration 004 directives`.

## Task 3: Implement the Rust bounded-resource foundation

**Files:** `src/resource/*`, `src/lib.rs`, `tests/resource_foundation.rs`

- [ ] Add failing deterministic tests for queue item/byte caps, FIFO admission, permit exhaustion, idempotent cancellation, owner retirement, and low-cardinality diagnostics:

  ```rust
  #[test]
  fn retiring_an_owner_cancels_and_drains_exactly_once() {
      let owner = ResourceOwner::new(ResourceBounds::new(2, 8).unwrap());
      owner.queue().try_push(4, "first").unwrap();
      owner.queue().try_push(4, "second").unwrap();
      assert_eq!(owner.retire(), Retirement { canceled: true, drained_items: 2, drained_bytes: 8 });
      assert_eq!(owner.retire(), Retirement::already_retired());
      assert_eq!(owner.queue().try_push(1, "late"), Err(ResourceError::Retired));
  }
  ```

- [ ] Run `rtk cargo test --test resource_foundation`; record failure because `resource` is not exported.
- [ ] Implement executor-neutral primitives. Queue admission must reserve bytes before storing the item and release exactly once on pop/drain:

  ```rust
  pub struct BoundedQueue<T> {
      bounds: ResourceBounds,
      retained_bytes: usize,
      items: VecDeque<BoundedItem<T>>,
      retired: bool,
  }

  impl<T> BoundedQueue<T> {
      pub fn try_push(&mut self, bytes: usize, value: T) -> Result<(), ResourceError> {
          if self.retired { return Err(ResourceError::Retired); }
          let next = self.retained_bytes.checked_add(bytes).ok_or(ResourceError::BytesExceeded)?;
          if self.items.len() == self.bounds.max_items() { return Err(ResourceError::ItemsExceeded); }
          if next > self.bounds.max_bytes() { return Err(ResourceError::BytesExceeded); }
          self.retained_bytes = next;
          self.items.push_back(BoundedItem { bytes, value });
          Ok(())
      }
  }
  ```

  `CancellationFlag`, `PermitPool`, and `ResourceOwner` use atomics/mutexes only where cross-thread ownership requires them; none spawn tasks or expose dispatch/RPC methods. Export `pub mod resource;` from `lib.rs` with complete docs.

- [ ] Run focused tests, `rtk cargo test --test security_boundaries`, format, and Clippy without blanket warning denial.
- [ ] Commit: `feat(resource): add bounded lifecycle primitives`.

## Task 4: Implement the browser bounded-resource foundation

**Files:** `browser/src/features/bounded.ts`, `browser/src/lifecycle/resources.ts`, `browser/tests/bounded-resources.test.ts`

- [ ] Add failing fake-clock tests for queue bytes/items, permit fairness, cancellation, suspension, resume, retirement, and exact disposer counts:

  ```ts
  it("retires queued bytes and active permits exactly once", () => {
    const owner = new BoundedOwner({ maxItems: 2, maxBytes: 8, maxActive: 1 });
    expect(owner.enqueue("a", 4)).toBe("accepted");
    const lease = owner.acquire();
    expect(lease).not.toBeNull();
    expect(owner.retire()).toEqual({
      drainedItems: 1,
      drainedBytes: 4,
      releasedPermits: 1,
    });
    expect(owner.retire()).toEqual({
      drainedItems: 0,
      drainedBytes: 0,
      releasedPermits: 0,
    });
    lease?.dispose();
    expect(owner.snapshot().active).toBe(0);
  });
  ```

- [ ] Run `rtk npm --prefix browser test -- bounded-resources.test.ts`; record failure because the owner does not exist.
- [ ] Implement closed primitives and extend lifecycle resource labels:

  ```ts
  export type CoreResourceKind =
    | "controller"
    | "extension"
    | "listener"
    | "observer"
    | "scheduler"
    | "signal"
    | "timer"
    | "transition"
    | "transport";
  export type FeatureResourceKind = "upload" | "stream" | "poll";
  export type ResourceKind = CoreResourceKind | FeatureResourceKind;

  export class BoundedOwner<T> {
    #state: "active" | "suspended" | "retired" = "active";
    #queuedBytes = 0;
    #active = 0;
    readonly #queue: Array<{ value: T; bytes: number }> = [];

    enqueue(
      value: T,
      bytes: number,
    ): "accepted" | "items_exceeded" | "bytes_exceeded" | "retired" {
      if (this.#state === "retired") return "retired";
      if (this.#queue.length === this.#limits.maxItems) return "items_exceeded";
      if (bytes > this.#limits.maxBytes - this.#queuedBytes)
        return "bytes_exceeded";
      this.#queue.push({ value, bytes });
      this.#queuedBytes += bytes;
      return "accepted";
    }
  }
  ```

  Keep callbacks typed by the owning feature; the foundation has no string method dispatch, DOM install, or authority mutation.

- [ ] Run unit tests, lifecycle tests, typecheck, lint, and format check.
- [ ] Commit: `feat(browser): add bounded feature resources`.

## Task 5: Add one closed optional-feature host to the core runtime

**Files:** `browser/src/features/{contract,host,global,producer,stimulus}.ts`,
runtime/island/lifecycle modules, `browser/tests/{feature-host,feature-import-graph}.test.ts`

**Implementation re-anchor (2026-08-24):** Measured production builds proved
that placing the two-slot registry in the universal artifact exceeds the
existing 45 KiB Brotli cap even after controller normalization moved outward.
The checked boundary is therefore one frozen, exact, versioned lifecycle-driver
attachment in core. Core records the attachment and lifecycle state before any
callback, owns one driver claim per validated island, replays its bounded active
island map on late first attachment, constructs only narrow island-bound ports,
orders start/suspend/resume/retire/dispose, rejects stale ports, and isolates
fixed redacted diagnostics. The trusted optional driver owns the fixed
`uploads`/`async` two-slot registry, slot/version/range checks, per-slot claims,
raw accessor inspection, directive scanning with nested-island filtering,
controller/disposer graphs, and late-second-slot replay across the bounded
active ports already delivered by core. Name-specific `defineUploadsFeature`
and `defineAsyncFeature` adapters register through that shared driver; classic
and ESM entry points share its bounded pending surface before or after boot.
`BootstrapOptions.features` retains its platform-capability override meaning.
The same driver owns one separate Stimulus adapter singleton; it is not a third
feature slot. Core emits validated, exactly-once before-morph,
after-successful-morph, abort, retire, suspend/resume, and document-dispose
edges, while the adapter owns application/definition validation and continuity
records. Preserve `BootstrapOptions.stimulus`, unchanged `boot({ stimulus })`,
the existing exported structural types, and both ESM/classic behavior. Missing
adapter registration emits one bounded Stimulus-unavailable diagnostic without
disabling ordinary Live; duplicate adapter loading is idempotent.
The producer brand is only a format marker, not authentication: exhaustive core
driver-envelope validation plus fresh-render admission, immutable identity, and
presentation-signal ownership checks remain the authority boundary. A forged
driver receives no action, effect, HTML, snapshot, endpoint, module-loader,
module-URL, or import authority. The page-local Stimulus `Application` and
definitions deliberately supplied through unchanged `boot({ stimulus })` are
optional adapter configuration, not executable-resolution authority; the
same-origin JavaScript realm is not an authentication boundary.
The optional driver retains at most 256 active island ports per document;
additional optional-capability admission fails closed with one
`resource_exhausted` diagnostic and retirement releases capacity without
affecting ordinary Live.
This replaces the illustrative raw-object construction and direct per-feature
core registration below; its behavioral assertions remain binding at the
driver/optional-registry boundary.

- [ ] Add failing tests proving duplicate registration is idempotent only for the same object/version, classic registration works both before and after core boot, incompatible features fail closed, one island gets one feature owner, retirement disposes once, and feature code cannot queue actions:

  ```ts
  const feature = defineUploadsFeature({
    connectDocument(context) {
      return {
        connectIsland: vi.fn(() => ({ dispose: disposeIsland })),
        dispose: disposeDocument,
      };
    },
  });
  expect(host.register(feature)).toBe("registered");
  expect(host.register(feature)).toBe("already_registered");
  expect("enqueueAction" in capturedContext).toBe(false);
  ```

- [ ] Run `rtk npm --prefix browser test -- feature-host.test.ts`; record failure because the feature host is absent.
- [ ] Define the narrow driver contract and wire it into `SuprnovaLiveRuntime`, `DocumentRuntime.#connect`, `IslandRecord.onDispose`, suspend/resume, and document disposal:

  ```ts
  export interface RuntimeFeatureIslandPort {
    readonly element: Element;
    readonly identity: IslandExtensionIdentity;
    enqueueFreshRender(
      reason: "poll" | "stream",
    ): "queued" | "coalesced" | "retired";
    onDispose(dispose: () => void): void;
  }

  export interface RuntimeFeatureDefinition {
    connectDocument(
      context: RuntimeFeatureDocumentContext,
    ): FeatureDocumentController;
  }

  export interface RuntimeFeature {
    readonly name: "uploads" | "async";
    readonly capabilityVersion: 1;
    readonly coreRange: Readonly<{ minimum: string; maximumExclusive: string }>;
    // Opaque normalized registration; construct through a closed producer.
  }

  export function defineUploadsFeature(
    definition: RuntimeFeatureDefinition,
  ): RuntimeFeature;
  ```

  Core exposes only frozen validated-island identity, scheduler-mediated fresh
  rendering, presentation-signal writes after current ownership validation,
  and bounded lifecycle edges. Optional code scans directives within the
  validated root and owns feature/controller resources. Neither side exposes
  action construction, response commit, raw snapshot mutation, HTML
  replacement, effect lookup, endpoints, or arbitrary JavaScript lookup.

- [ ] Implement the shared ESM/classic optional driver surface through
  `Symbol.for("suprnova.live.features.v1")` with one exact driver attachment and
  a bounded two-name registry behind it. Adopt it during `boot`, clear adopted
  pending entries, and make same-driver repetition idempotent while a different
  driver conflicts. Optional ESM registrations use the same surface rather than
  overloading `BootstrapOptions.features`.
- [ ] Move `stimulus/bridge.ts` and `stimulus/lifecycle.ts` behind the optional
  driver singleton without importing `@hotwired/stimulus`. Prove core lifecycle
  ordering, stale-port inertness, abort/retire cleanup, exception isolation, and
  unchanged application-supplied Stimulus boot behavior before and after morphs.
- [ ] Run feature-host, discovery, lifecycle, scheduler, and public API tests plus typecheck/lint.
- [ ] Commit: `feat(browser): add closed optional feature host`.

## Task 6: Produce deterministic role-typed optional artifacts

**Files:** optional entry points, `browser/src/assets.ts`, build scripts, `browser/package.json`, `browser/tests/optional-artifacts.test.ts`

- [ ] Add a failing build test requiring exactly ten output files (eight scripts,
  one declaration file, and one manifest), eight executable asset roles,
  independent hashes, compatibility metadata, and reproducibility:

  ```ts
  expect(manifest.assets.map((asset) => asset.role).sort()).toEqual([
    "async-classic",
    "async-esm",
    "core-classic",
    "core-esm",
    "stimulus-classic",
    "stimulus-esm",
    "uploads-classic",
    "uploads-esm",
  ]);
  expect(
    manifest.assets.every(
      (asset) => asset.compatible_core === ">=0.1.0 <0.2.0",
    ),
  ).toBe(true);
  ```

- [ ] Run `rtk npm --prefix browser run build:check`; record failure because only core assets exist.
- [ ] Add inert feature factories to the four upload/async entry points so the
  build contract lands before their behavior:

  ```ts
  export const uploadsFeature: RuntimeFeature = createUnavailableFeature(
    "uploads",
    1,
  );
  export default uploadsFeature;
  ```

  ```ts
  import { registerClassicFeature } from "./features/global.js";
  import { uploadsFeature } from "./entry-uploads-esm.js";
  registerClassicFeature(globalThis, uploadsFeature);
  ```

  The async pair follows the same pattern. ESM exports a feature registration;
  classic self-registers. Add the Stimulus ESM/classic pair as the optional
  driver singleton that preserves `boot({ stimulus })` and the public structural
  types. No optional entry imports a core runtime value, and no Stimulus entry
  imports `@hotwired/stimulus`.

- [ ] Refactor `build.mjs` around this closed output table and attach `capability`, `capabilityVersion`, and `compatibleCore` to each manifest record:

  ```js
  const OUTPUTS = Object.freeze([
    {
      file: "suprnova-live.classic.js",
      role: "core-classic",
      format: "iife",
      entryPoint: "src/entry-classic.ts",
    },
    {
      file: "suprnova-live.esm.js",
      role: "core-esm",
      format: "esm",
      entryPoint: "src/entry-esm.ts",
    },
    {
      file: "suprnova-live.stimulus.classic.js",
      role: "stimulus-classic",
      format: "iife",
      entryPoint: "src/entry-stimulus-classic.ts",
    },
    {
      file: "suprnova-live.stimulus.esm.js",
      role: "stimulus-esm",
      format: "esm",
      entryPoint: "src/entry-stimulus-esm.ts",
    },
    {
      file: "suprnova-live.uploads.classic.js",
      role: "uploads-classic",
      format: "iife",
      entryPoint: "src/entry-uploads-classic.ts",
    },
    {
      file: "suprnova-live.uploads.esm.js",
      role: "uploads-esm",
      format: "esm",
      entryPoint: "src/entry-uploads-esm.ts",
    },
    {
      file: "suprnova-live.async.classic.js",
      role: "async-classic",
      format: "iife",
      entryPoint: "src/entry-async-classic.ts",
    },
    {
      file: "suprnova-live.async.esm.js",
      role: "async-esm",
      format: "esm",
      entryPoint: "src/entry-async-esm.ts",
    },
  ]);
  ```

  Emit manifest schema 2 and declaration exports for core plus the three
  optional ESM registrations. Reject Idiomorph in optional metafiles. Reject
  `stimulus/bridge.ts` and `stimulus/lifecycle.ts` in core/upload/async
  metafiles, and reject `@hotwired/stimulus` in every production metafile. The
  server-side manifest resolver chooses script URLs/roles; no directive or
  island attribute is accepted as a module URL.

- [ ] Update `package.json` exports for `./stimulus`, `./uploads`, and `./async`,
  keep core side effects limited to classic, and mark all optional classic
  artifacts as side effects. Run two-build byte equality, manifest validation,
  package tests, typecheck, and build.
- [ ] Commit: `build(browser): emit optional feature artifacts`.

## Task 7: Enforce artifact ceilings and unaffected core behavior

**Files:** `browser/scripts/check-budget.mjs`, budget/package tests, `scripts/gate.sh`

- [ ] Add failing budget assertions for per-role Brotli ceilings and a core-only runtime boot with optional modules absent:

  ```js
  const limits = new Map([
    ["core-esm", 45 * 1024],
    ["core-classic", 45 * 1024],
    ["stimulus-esm", 8 * 1024],
    ["stimulus-classic", 8 * 1024],
    ["uploads-esm", 20 * 1024],
    ["uploads-classic", 20 * 1024],
    ["async-esm", 16 * 1024],
    ["async-classic", 16 * 1024],
  ]);
  for (const asset of manifest.assets) {
    const compressed = brotliCompressSync(
      await readFile(resolve(browserRoot, "dist", asset.file)),
      options,
    );
    if (compressed.byteLength > limits.get(asset.role))
      throw new Error(`artifact_budget:${asset.role}`);
  }
  ```

- [ ] Run `rtk npm --prefix browser run budget`; record failure until role-aware budget checks are implemented.
- [ ] Make budget output print every role/size and fail if a required role is absent, duplicated, over budget, or incompatible. Add the focused optional-artifact, core bootstrap, CSP, lifecycle, and reproducibility commands to `scripts/gate.sh` without weakening Iteration 001–003 gates.
- [ ] Run `rtk npm --prefix browser run build`, `build:check`, `budget`, unit tests, and `rtk proxy tests/gate_contract.sh`.
- [ ] Commit: `test: gate optional artifact contracts`.

## Task 8: Verify and hand off the shared foundation

**Files:** all files in this plan

- [ ] Run the complete focused foundation gate:

  ```bash
  rtk cargo fmt --all -- --check
  rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features
  rtk env CARGO_INCREMENTAL=0 cargo test --test resource_foundation --test golden_fixtures --test browser_contract_properties
  rtk npm --prefix browser run generate:check
  rtk npm --prefix browser run format:check
  rtk npm --prefix browser run lint
  rtk npm --prefix browser run typecheck
  rtk npm --prefix browser test -- feature-host.test.ts bounded-resources.test.ts optional-artifacts.test.ts golden-fixtures.test.ts
  rtk npm --prefix browser run build:check
  rtk npm --prefix browser run budget
  rtk git diff --check
  ```

- [ ] Review every changed diagnostic for low-cardinality codes and secret/raw-payload exclusion. Confirm `SUPPORTED_PROTOCOL_VERSIONS` remains `[1, 2]`, optional absence leaves ordinary Live usable, and no feature port exposes generic dispatch, HTML install, snapshot mutation, or action invocation.
- [ ] Check `rtk git status --short`, inspect the full diff, and commit any verification-only corrections as `chore: close iteration 004 foundation gate`.

## Definition-of-done coverage

- DOD 1: Tasks 3–5 establish shared bounded ownership and exact retirement.
- DOD 24: Tasks 4–5 connect suspend/resume/bfcache/document retirement once.
- DOD 26–28: Tasks 1–2 and 5–7 establish v4 grammar, independent protocol capability metadata, deterministic optional assets, compatibility, CSP-safe registration, and hard byte caps.
- DOD 30, 35, 37: Tasks 1–8 add cross-language/property/build evidence, preserve earlier gates, avoid blanket warning denial, and keep Suprnova/Magnetar untouched.

## Plan self-review checklist

- [ ] Every implementation task starts with a failing focused test and names its expected failure.
- [ ] Every created or modified path is listed and every behavior-changing step contains executable code or an exact command.
- [ ] The feature host exposes refresh/presentation/lifecycle capability only; it cannot become a generic RPC seam.
- [ ] Core and optional artifacts are independently measurable and optional code is absent from core metafiles.
- [ ] Fixture v4 extends capabilities without renumbering Live action/morph protocol v2.
- [ ] No unfinished marker, fake vendor adapter, persistent browser storage, RenderCache, component-library, or Suprnova integration work appears in this plan.
