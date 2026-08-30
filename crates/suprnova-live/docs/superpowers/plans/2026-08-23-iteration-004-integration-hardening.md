# Iteration 004 Integration, Hardening, and Release Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate uploads and asynchronous updates through the reference host, checker, fixtures, browser matrix, adversarial suites, hard performance budgets, implementation documentation, and one unattended Iteration 004 gate while preserving every Iteration 001–003 guarantee.

**Architecture:** Exercise only production-built core/Stimulus/upload/async artifacts against a thin Rust reference host that calls the actual engine services for deterministic HTTP chunk, direct-transfer, SSE, WebSocket, and poll scenarios. Node owns static scenario pages, assets, launcher orchestration, and assertions only; it never reimplements upload, quarantine, authorization, continuity, or stream semantics. Cross-language fixtures and the Askama checker remain the authoring authority; the host is explicitly test infrastructure, not Suprnova product integration. Benchmark upload control separately from provider/file/scanner/application time, measure browser work in pinned environments, and gate hard caps plus the existing 15-percent regression rule. One root gate composes all earlier and new checks without blanket warning denial.

**Tech Stack:** Rust 1.91.1, existing benchmark harnesses and test-support crate, exact test-support dependencies Axum 0.7.9, Tokio 1.53.1, and futures-util 0.3.34, strict TypeScript 6.0.3, Node static-scenario launcher, Playwright 1.62.1 Chromium/Firefox/WebKit, axe-core 4.13.0, browser performance/heap instrumentation, deterministic clocks/randomness/network schedules.

---

## Dependencies and execution rules

- This is Plan 4 of 4. Complete and verify the shared foundation, upload, and async-update plans first.
- Work only in `/home/shawn/workspace2/suprnova-live/.worktrees/iteration-004-uploads-async`; never push without explicit developer authorization.
- Start every shell command with `rtk`; use `apply_patch` for hand edits; do not use blanket `-D warnings`.
- Keep benchmark B1/S1 environments and qualification rules exactly aligned with `docs/specs/suprnova-live/19-developer-tooling-and-testing.md` and Iteration 004.
- Browser automation tools may diagnose behavior, but checked-in Playwright and benchmark evidence are the release authority.
- Do not modify Suprnova or Magnetar. Do not present reference host/adapters as product integration.
- Before Task 1, record the read-only `git status --short`, branch, and HEAD for Suprnova and Magnetar. Compare those baselines during Task 9 so unrelated work already in progress is preserved and never misattributed to Iteration 004.

## File structure

### Create

- `browser/test-host/faults.mjs`
- `crates/suprnova-live-test-support/src/reference_host/mod.rs`
- `crates/suprnova-live-test-support/src/reference_host/artifacts.rs`
- `crates/suprnova-live-test-support/src/reference_host/uploads.rs`
- `crates/suprnova-live-test-support/src/reference_host/async_updates.rs`
- `crates/suprnova-live-test-support/src/reference_host/faults.rs`
- `crates/suprnova-live-test-support/src/bin/suprnova-live-reference-host.rs`
- `crates/suprnova-live-test-support/tests/reference_host.rs`
- `browser/e2e/iteration-004-integration.spec.ts`
- `browser/e2e/iteration-004-adversarial.spec.ts`
- `browser/e2e/iteration-004-lifecycle.spec.ts`
- `browser/e2e/iteration-004-accessibility.spec.ts`
- `browser/benchmarks/upload-workloads.ts`
- `browser/benchmarks/async-workloads.ts`
- `browser/benchmarks/baselines/upload-budget-v1.json`
- `browser/benchmarks/baselines/async-budget-v1.json`
- `browser/scripts/run-upload-budget.mjs`
- `browser/scripts/run-async-budget.mjs`
- `benches/upload_framework_budget.rs`
- `benches/async_framework_budget.rs`
- `scripts/run-upload-budget.sh`
- `scripts/run-async-budget.sh`
- `tests/iteration_004_conformance.rs`
- `tests/iteration_004_adversarial.rs`
- `tests/iteration_004_exhaustion.rs`
- `docs/implementation/uploads.md`
- `docs/implementation/async-updates.md`
- `docs/implementation/iteration-004-operations.md`

### Modify

- `Cargo.toml`
- `Cargo.lock`
- `crates/suprnova-live-test-support/Cargo.toml`
- `crates/suprnova-live-test-support/src/lib.rs`
- `crates/suprnova-live-test-support/src/host.rs`
- `THIRD_PARTY_LICENSES.md`
- `browser/package.json`
- `browser/playwright.config.ts`
- `browser/test-host/server.mjs`
- `browser/test-host/scenarios.mjs`
- `browser/benchmarks/schema.ts`
- `browser/benchmarks/workloads.ts`
- `browser/benchmarks/runner.ts`
- `browser/scripts/check-budget.mjs`
- `tests/checker_positive.rs`
- `tests/checker_negative.rs`
- `tests/checker_regressions.rs`
- `tests/documentation_contract.sh`
- `scripts/check-implementation-docs.mjs`
- `scripts/gate.sh`
- `README.md`
- `docs/specs/suprnova-live/conventions.md`

## Task 1: Serve exact production artifacts and deterministic real transports

**Files:** test-host modules, package files, artifact/build tests

- [ ] Add failing host tests that require the manifest-selected
  core/Stimulus/upload/async ESM and classic files, exact hashes/SRI/content
  headers, real chunked request bodies, direct instructions, SSE, WebSocket,
  poll, controlled faults, and deterministic shutdown:

  ```ts
  it("serves only manifest-owned production scripts", async () => {
    const manifest = await host.assets();
    for (const asset of manifest.assets) {
      const response = await fetch(host.url(asset.file));
      expect(response.headers.get("cache-control")).toBe(asset.cache_control);
      expect(sha256(await response.arrayBuffer())).toBe(asset.sha256);
    }
    expect((await fetch(host.url("/src/runtime/runtime.ts"))).status).toBe(404);
  });
  ```

- [ ] Run the test-host/package/build suite; record failure because Iteration 004 routes and optional artifacts are not wired.
- [ ] Add exact test-support-only dependencies, with the standard provenance,
  license, lockfile, feature, and MSRV review. Reuse the versions already pinned
  by the read-only Suprnova workspace where applicable:

  ```toml
  axum = { version = "=0.7.9", features = ["ws"] }
  futures-util = { version = "=0.3.34", features = ["sink"] }
  http-body-util = "=0.1.3"
  hyper = { version = "=1.9.0", features = ["client", "http1"] }
  hyper-util = { version = "=0.1.20", features = ["client-legacy", "http1", "tokio"] }
  tokio = { version = "=1.53.1", features = ["fs", "io-util", "macros", "net", "rt-multi-thread", "sync", "test-util", "time"] }
  ```

  Keep these dependencies inside `suprnova-live-test-support`; the engine gains
  no executor, HTTP server, filesystem, or WebSocket dependency.

- [ ] Implement the browser-facing Rust host with a closed route table:

  ```rust
  pub const CREATE_UPLOAD: &str = "/__live/uploads";
  pub const UPLOAD_CHUNK: &str = "/__live/uploads/:handle/chunks/:part";
  pub const UPLOAD_STATUS: &str = "/__live/uploads/:handle";
  pub const UPLOAD_COMPLETE: &str = "/__live/uploads/:handle/complete";
  pub const UPLOAD_CANCEL: &str = "/__live/uploads/:handle/cancel";
  pub const EXAMPLE_REACQUIRE: &str = "/example/uploads/:handle/reacquire";
  pub const POLL: &str = "/__live/async/poll";
  pub const TRANSPORT_CREATE: &str = "/__live/async/transports";
  pub const TRANSPORT_MEMBERSHIP: &str =
      "/__live/async/transports/:transport/subscriptions/:subscription";
  pub const SSE: &str = "/__live/async/sse/:transport";
  pub const WEBSOCKET: &str = "/__live/async/ws";
  ```

  The reacquisition example is an authenticated application-owned route outside
  `/__live/`. Poll always invokes ordinary fresh render; it never names an
  action. SSE uses authenticated membership control plus one physical document
  stream. WebSocket accepts bounded subscribe/unsubscribe control frames on one
  physical document connection and performs strict Origin validation before
  upgrade.

- [ ] Route every dynamic request into the actual Rust upload/authorization/
  sequence/continuity services and test-support adapters from Plans 2 and 3.
  Stream request chunks through `TokioFileQuarantineStore` with configured
  byte/part limits. Direct-provider scenarios return constrained reference
  instructions. SSE and WebSocket encode the actual async envelope over
  deterministic engine-backed event sources. The host may select only compiled
  server-owned fault schedules; query parameters never select paths, commands,
  credentials, or arbitrary payloads.

- [ ] Keep Node limited to the static production artifacts and deterministic
  scenario pages it already serves. The Rust host is the browser-facing origin
  for dynamic routes and may reverse-proxy allowlisted static GET/HEAD requests
  to Node. JavaScript must not implement upload state, grants, quarantine,
  authorization, sequence, continuity, or dynamic response fixtures.

- [ ] Validate the artifact manifest before proxying, bind deterministic ports,
  close sockets/files/timers on teardown, and expose inspection counters that
  contain no grants/tokens/raw payloads.
- [ ] Run host, artifact, shutdown, and production-source exclusion tests; commit `test(host): serve iteration 004 production scenarios`.

## Task 2: Close Askama checker and cross-language conformance gaps

**Files:** checker tests, v4 fixtures/generator outputs, `tests/iteration_004_conformance.rs`

- [ ] Add failing positive/negative/regression cases for every new directive value, modifier, role, conflict, owner, capability version, accessibility obligation, static branch, and dynamic/unproved branch:

  ```rust
  #[test]
  fn checked_upload_and_stream_markup_proves_roles_and_ownership() {
      assert_checked(r#"
          <input type="file" live:upload="avatar" multiple>
          <div live:progress="avatar" role="progressbar"></div>
          <button live:upload.cancel="avatar">Cancel</button>
          <section live:stream="orders" live:poll.visible.30s></section>
      "#);
  }

  #[test]
  fn dynamic_feature_markup_is_never_reported_as_statically_proved() {
      assert_unproved("<input {{ attrs }}>", "dynamic_live_contract");
  }
  ```

- [ ] Run checker and golden fixture suites; record the first missing semantic rule or parity failure.
- [ ] Implement only checker rules generated or derived from v4 contracts. Require file input for `live:upload`, scoped progress/control ownership, accessible progress naming, declared upload field/subscription/event/signal metadata, legal poll/stream mode combinations, and static capability compatibility. Keep dynamic markup explicitly unproved.
- [ ] Add one cross-language conformance test that loads every v4 case through Rust and TypeScript codecs/parsers and compares canonical disposition/code/state/position. Preserve the fixture's separation between wire `operations`/`codec_cases` and internal `transition_cases`; add an exhaustive typed mapping rather than exposing internal transition names on the wire. Run generation drift, checker suites, fixtures, and protocol v1/v2 compatibility.
- [ ] Commit: `test: close iteration 004 authoring conformance`.

## Task 3: Build the real-browser functional and lifecycle matrix

**Files:** four Iteration 004 Playwright specs, test-host scenarios, Playwright config

- [ ] Add failing data-driven tests for core-only, uploads-only, async-only, and both-feature pages in ESM/classic form under strict CSP:

  ```ts
  for (const scriptKind of ["module", "classic"] as const) {
    test(`${scriptKind} composes optional features once`, async ({ page }) => {
      await page.goto(`/iteration-004/both?script=${scriptKind}`);
      await page.setInputFiles("[live\\:upload='avatar']", FILE_16_MIB);
      await expect(page.locator("[live\\:progress='avatar']")).toHaveAttribute(
        "data-live-upload-state",
        "ready",
      );
      await expect(page.locator("[live\\:stream='orders']")).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );
      expect(await resourceCounts(page)).toMatchObject({
        upload: 1,
        documentTransports: 1,
        logicalSubscriptions: 1,
      });
    });
  }
  ```

- [ ] Run Chromium integration/lifecycle specs; record failures for every unwired scenario.
- [ ] Implement deterministic scenarios for native selection/transfer/finalize, direct conformance transfer, SSE, WebSocket, polling, hybrid gaps, ordinary action during transfer/outage, morph preservation/replacement, navigation, offline, pagehide/freeze/resume/pageshow/bfcache, and shutdown. Include multiple islands sharing one document transport, logical membership removal without physical teardown, and transport-wide reconnect. Assert no correctness sleep; drive controlled clocks/network steps through host/test ports.
- [ ] Prove real bfcache restoration with `PageTransitionEvent.persisted === true`: persisted `pagehide` closes every physical transport and timer, persisted `pageshow` obtains current authorization and opens a new transport, and no old buffered event may apply. Synthetic lifecycle hooks remain useful unit evidence but cannot satisfy this browser requirement.
- [ ] Add accessible-name, keyboard, focus, error association, throttled live-region, reduced-motion, and axe checks. Run Chromium/Firefox/WebKit for ESM/classic and missing/incompatible optional artifacts.
- [ ] Commit: `test(browser): integrate uploads and async updates`.

## Task 4: Lock the complete adversarial and exhaustion matrix

**Files:** Rust adversarial/exhaustion tests, Playwright adversarial spec, fault scenarios

- [ ] Add failing table-driven Rust and browser cases for every DOD 31 attack/race and for scoped exhaustion that leaves unrelated Live work usable:

  ```rust
  #[tokio::test]
  async fn adversarial_cases_have_typed_bounded_dispositions() {
      for case in adversarial_cases() {
          let outcome = case.execute().await;
          assert!(outcome.is_typed());
          assert!(outcome.retained_bytes() <= case.bound());
          assert!(case.unrelated_island_remains_usable().await);
          assert!(!outcome.safe_diagnostic().contains(case.secret_sentinel()));
      }
  }
  ```

- [ ] Run adversarial/exhaustion suites; record the first missing typed disposition.
- [ ] Cover forged/cross-scope handles, grant/token sentinel leaks, oversized/truncated/reordered chunks/messages, malformed and adversarial media headers, duplicate completion, cancel/finalize, expire/finalize, scan timeout, provider partial failure, replay overflow, revoked authorization, cross-site WebSocket attempts, missing/malformed/wildcard/unapproved Origin headers, fanout pressure, reconnect storms, late events, and retirement. Every fault maps to a closed error/recovery code; unknown failures fail the dependent feature closed while normal routes/actions continue.
- [ ] Run security boundaries, hostile context, error redaction, upload/async adversarial suites, and browser CSP/leak tests.
- [ ] Commit: `test(security): harden iteration 004 boundaries`.

## Task 5: Implement and hard-gate U4/16

**Files:** Rust/browser upload benchmark files, scripts, baselines, Cargo/package scripts

- [ ] Add failing benchmark-schema tests requiring workload ID, artifact hash, B1/S1 environment, bounds, warmup, samples, p50/p95, retained bytes, chunk buffers, queue/concurrency, and qualified/unqualified classification:

  ```rust
  const WORKLOAD: &str = "U4/16";
  const FILES: usize = 4;
  const FILE_BYTES: usize = 16 * 1024 * 1024;
  const CHUNK_BYTES: usize = 256 * 1024;
  const MAX_BROWSER_CHUNKS_PER_TRANSFER: usize = 2;
  const MAX_SERVER_CHUNKS_PER_TRANSFER: usize = 2;
  const MAX_BROWSER_MANAGER_BYTES: usize = 256 * 1024;
  const MAX_SERVER_MANAGER_BYTES: usize = 512 * 1024;
  ```

- [ ] Run upload budget scripts; record failure because the workload/baseline is absent.
- [ ] Implement `benches/upload_framework_budget.rs` with a null-body/null-provider/null-scanner/application port so timing measures verified control admission, conditional transition, idempotency, and response encoding only. Exclude body I/O, provider, scan, and application validation by construction; assert exclusion counters remain zero. Require control framework p95 `<= 2 ms` on S1.
- [ ] Implement the B1 browser workload using four synthetic 16 MiB `File` objects, 256 KiB slices, four active transfers, and controlled immediate transport receipts. Count live chunk buffers and manager-owned bytes through benchmark-only observers compiled outside production artifacts. Require at most two chunks per active transfer plus 256 KiB manager overhead and progress application p95 `<= 16 ms`.
- [ ] Add `upload-budget-v1.json`, schema validation, release qualification, 15-percent regression comparison, artifact hash binding, `run-upload-budget.sh`, and package/root scripts. Run both budgets and commit `perf: gate U4/16 upload workload`.

## Task 6: Implement and hard-gate E100/1K and R100

**Files:** Rust/browser async benchmark files, scripts, baselines, schema

- [ ] Add failing schema/workload tests for both IDs and every bound:

  ```ts
  export const E100_1K = Object.freeze({
    subscriptions: 100,
    documentTransports: 1,
    events: 1_000,
    payloadBytes: 1_024,
    durationMs: 10_000,
    refreshRatio: 0.1,
    maxRetainedBytesPerSubscription: 8 * 1024,
    maxDocumentEvents: 64,
    maxDocumentBytes: 256 * 1024,
    maxDispatchP95Ms: 8,
  });

  export const R100 = Object.freeze({
    subscriptions: 100,
    reconnectHandshakes: 1,
    maxConcurrentHandshakesPerOrigin: 8,
    maxRetainedBytesAfterCurrent: 12 * 1024,
  });
  ```

- [ ] Run async budget scripts; record failure because workloads/baselines are absent.
- [ ] Implement E100/1K with 100 island subscriptions multiplexed over exactly one physical document transport, 1,000 ordered 1 KiB presentation events in a controlled ten-second timeline, and exactly 100 refresh invalidations. Require `<= 8 KiB` retained per subscription excluding native transport/DOM/current payload, document queue `<= 64` events and `<= 256 KiB`, dispatch p95 `<= 8 ms` on B1, and per-island refresh `<= one queued + one in-flight`.
- [ ] Implement R100 by simultaneously removing continuity from all 100 subscriptions on that document. Record exactly one reconnect handshake, reconnect jitter, poll firing buckets, time to authoritative currentness, and retained bytes after currentness. Require no same-tick synchronized poll burst and return within the existing 12 KiB retained-runtime/island cap.
- [ ] Add a separate deterministic multi-document scheduler test that attempts more than eight simultaneous document-transport handshakes for one origin and proves the global active count never exceeds eight. Do not describe this as 100 physical connections or use it as the R100 topology.
- [ ] Add browser/Rust baselines, artifact hash/environment/sample metadata, hard caps, 15-percent regression checks, and release qualification. Run both workloads and commit `perf: gate async continuity workloads`.

## Task 7: Document authoring, policy, operations, and integration boundaries

**Files:** implementation docs, README indexes, documentation checker/tests

- [ ] Add failing documentation-contract assertions for every required section:

  ```js
  const required = new Map([
    [
      "docs/implementation/uploads.md",
      [
        "Handle and grant",
        "Provider modes",
        "Quarantine and scanning",
        "Finalization and compensation",
        "Current-document resume",
        "Cleanup",
      ],
    ],
    [
      "docs/implementation/async-updates.md",
      [
        "Event schemas",
        "Subscription authorization",
        "Polling and push modes",
        "Continuity",
        "Degraded freshness",
        "Backpressure",
      ],
    ],
    [
      "docs/implementation/iteration-004-operations.md",
      [
        "Artifacts",
        "Limits",
        "Observability",
        "Benchmarks",
        "Reference-host boundary",
        "Suprnova integration boundary",
      ],
    ],
  ]);
  ```

- [ ] Run `rtk proxy tests/documentation_contract.sh` and `rtk node scripts/check-implementation-docs.mjs`; record failure because Iteration 004 docs are absent.
- [ ] Write complete authoring examples using Askama-compatible HTML and Rust metadata. Explain handle versus grant, quotas, file/direct-provider semantics, quarantine/validation/scanning policies, explicit finalization/compensation, current-document resume/reacquisition, cleanup, typed events, signed subscriptions, poll/push/hybrid continuity, degraded states, backpressure, artifact selection, testing, and observability. State explicitly that the reference host and direct adapter are conformance tools, not Suprnova/vendor integration.
- [ ] Update the root README implementation-document links and conventions with Iteration 004 artifact/protocol versions and the no-blanket-`-D warnings` rule. Run docs checks, link checks already in the gate, and spec checker. Do not invent `docs/implementation/README.md`; that index does not exist.
- [ ] Commit: `docs: explain uploads and asynchronous updates`.

## Task 8: Compose the unattended Iteration 004 gate

**Files:** `scripts/gate.sh`, gate-contract tests, package/Cargo scripts

- [ ] Add a failing shell contract that requires every unaffected old phase plus the new fixture, upload, async, reference-host, browser-matrix, fuzz-build, and reduced deterministic budget phases in ordinary mode; requires full qualified U4/16, E100/1K, and R100 only in release mode; and rejects blanket warning denial:

  ```bash
  rtk proxy tests/gate_contract.sh
  # Expected before implementation: missing required phase "U4/16 upload budget".
  ```

- [ ] Extend `scripts/gate.sh` with focused phases before broad suites and budgets after deterministic builds:

  ```bash
  phase "iteration 004 Rust boundaries"
  rtk env CARGO_INCREMENTAL=0 cargo test \
      --test iteration_004_conformance \
      --test iteration_004_adversarial \
      --test iteration_004_exhaustion

  phase "iteration 004 reduced deterministic budgets"
  rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-upload-budget.sh
  rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-async-budget.sh

  if [[ "${SUPRNOVA_LIVE_RELEASE:-0}" == "1" ]]; then
      phase "U4/16 qualified upload budget"
      rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-upload-budget.sh

      phase "E100/1K and R100 qualified async budgets"
      rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-async-budget.sh
  fi
  ```

  Keep `cargo clippy --workspace --all-targets --all-features` without `-D warnings`; continue reviewing emitted warnings. Ordinary mode mechanically verifies the same bounds and topology at a reduced deterministic scale. Release mode rejects unqualified B1/S1 evidence and cannot substitute reduced results for full qualification.

- [ ] Run gate contract, documentation contract, script syntax, package script tests, and `rtk git diff --check`.
- [ ] Commit: `ci: compose iteration 004 unattended gate`.

## Task 9: Run the complete gate and final adversarial self-audit

**Files:** all Iteration 004 implementation, tests, fixtures, artifacts, docs, and metadata

- [ ] Run the exact full gate without skipping phases:

  ```bash
  rtk env SUPRNOVA_LIVE_RELEASE=1 CARGO_INCREMENTAL=0 scripts/gate.sh
  ```

  Iteration 004 changes upload/provider/stream/resource-budget behavior, so its
  completion gate uses release mode. Record the final exit status and every
  explicitly unqualified environment result. Do not claim B1/S1 qualification
  unless the pinned environments actually ran and passed; an unavailable pinned
  environment leaves release qualification incomplete rather than falling back
  to reduced evidence.

- [ ] Run final repository checks:

  ```bash
  rtk git diff --check
  rtk git status --short
  rtk rg -n "TODO|TBD|unimplemented!|todo!|placeholder|stale iteration-003" src browser tests fixtures fuzz benches scripts docs Cargo.toml
  rtk git -C /home/shawn/workspace2/suprnova status --short
  rtk git -C /home/shawn/workspace2/suprnova-magnetar status --short
  ```

  Classify every text-search match instead of deleting legitimate historical/spec wording. The two external status commands are read-only; compare them with the recorded branch/HEAD/status baselines and confirm this iteration created no changes there.

- [ ] Audit the 37-item Iteration 004 DOD line by line against tests and gate evidence. Verify no persistent browser upload storage, generic RPC, streamed HTML, action-by-push, concrete vendor claim, RenderCache/component-library work, or framework integration entered scope.
- [ ] Inspect the complete branch diff and local commit list. Commit verification-only corrections as `chore: complete iteration 004 release gate`. Do not push.

## Definition-of-done coverage matrix

|   DOD | Primary evidence                                             |
| ----: | ------------------------------------------------------------ |
|     1 | Shared Plan Tasks 3–5; this plan Tasks 3–4, 9                |
|  2–10 | Upload Plan Tasks 1–7; this plan Tasks 1, 4, 9               |
| 11–14 | Upload Plan Tasks 8–9; this plan Tasks 1, 3–4                |
| 15–18 | Async Plan Tasks 1–5, 8; this plan Tasks 1–4                 |
| 19–24 | Async Plan Tasks 6–9; this plan Tasks 3–4                    |
| 25–27 | Shared/Upload/Async plans; this plan Tasks 2–4, 7            |
|    28 | Shared Plan Tasks 5–7; this plan Tasks 1, 3, 5–6             |
|    29 | This plan Tasks 1 and 3                                      |
| 30–31 | Upload/Async fuzz and security tasks; this plan Tasks 2–4, 9 |
|    32 | This plan Task 5                                             |
| 33–34 | This plan Task 6                                             |
|    35 | This plan Tasks 8–9                                          |
|    36 | This plan Task 7                                             |
|    37 | This plan Task 9                                             |

## Plan self-review checklist

- [ ] Every DOD item maps to a named test, workload, document, or gate phase.
- [ ] The thin Rust host owns every dynamic route and state machine and performs real network I/O; Node serves only static scenario pages/assets, and neither imports production TypeScript source.
- [ ] Upload handle reacquisition is demonstrated on an authenticated application route outside `/__live/`; no reserved Live reacquisition route exists.
- [ ] WebSocket Origin is rejected before upgrade unless same-origin or explicitly allowlisted with a separate non-cookie credential.
- [ ] Browser matrices cover core-only, each optional feature alone, both together, ESM/classic, strict CSP, missing/incompatible features, all three engines, accessibility, lifecycle, and bfcache.
- [ ] U4/16 excludes provider/file/scan/application time by construction and hard-gates both memory and p95 limits.
- [ ] E100/1K uses exactly one document transport for 100 logical subscriptions; R100 performs exactly one reconnect handshake; a separate multi-document test hard-gates the eight-per-origin handshake cap.
- [ ] Ordinary gates run reduced deterministic budget proofs; release mode alone runs and requires full qualified U4/16, E100/1K, and R100 evidence.
- [ ] Earlier iteration gates remain enabled and no blanket warning denial appears.
- [ ] Documentation distinguishes specifications, reference conformance infrastructure, eventual internal-crate integration, and explicitly deferred work.
- [ ] Final status is local-only and Suprnova/Magnetar remain read-only.
