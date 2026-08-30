#!/usr/bin/env node

import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  parseRustCandidates,
  scanRepository,
  scanSource,
} from "../scripts/check-correctness-delays.mjs";
import { iteration004VerificationSurfaces } from "../scripts/iteration-004-verification-surfaces.mjs";

function violationKinds(source, language = "javascript") {
  return scanSource({
    filePath: language === "rust" ? "fixture.rs" : "fixture.ts",
    language,
    source,
  }).map(({ kind }) => kind);
}

const rejectedJavaScriptMutations = [
  {
    name: "direct timeout-resolved promise",
    source: "await new Promise((resolve) => setTimeout(resolve, 10));",
    kind: "promise-timeout",
  },
  {
    name: "callback timeout-resolved promise",
    source: "await new Promise((resolve) => setTimeout(() => resolve(), 10));",
    kind: "promise-timeout",
  },
  {
    name: "window-qualified timeout-resolved promise",
    source:
      "await new Promise((resolve) => window.setTimeout(() => resolve(), 10));",
    kind: "promise-timeout",
  },
  {
    name: "Playwright wall-clock wait",
    source: "await page.waitForTimeout(10);",
    kind: "playwright-timeout",
  },
  {
    name: "global Promise and optional qualified timer",
    source:
      "await new globalThis.Promise((resolve) => globalThis?.setTimeout?.(resolve, 10));",
    kind: "promise-timeout",
  },
  {
    name: "aliased timer",
    source:
      "const later = window.setTimeout; await new Promise((resolve) => later(resolve, 10));",
    kind: "promise-timeout",
  },
  {
    name: "destructured timer",
    source:
      "const { setTimeout: later } = globalThis; await new Promise((resolve) => later(resolve, 10));",
    kind: "promise-timeout",
  },
  {
    name: "computed qualified timer alias",
    source:
      'const later = globalThis["setTimeout"]; await new Promise((resolve) => later(resolve, 10));',
    kind: "promise-timeout",
  },
  {
    name: "timers promises import alias",
    source:
      'import { setTimeout as delay } from "node:timers/promises"; await delay(10);',
    kind: "promise-timeout",
  },
  {
    name: "aliased Playwright wait",
    source: "const wait = page.waitForTimeout; await wait(10);",
    kind: "playwright-timeout",
  },
  {
    name: "destructured Playwright wait",
    source: "const { waitForTimeout: wait } = page; await wait(10);",
    kind: "playwright-timeout",
  },
  {
    name: "optional Playwright wait",
    source: "await page?.waitForTimeout?.(10);",
    kind: "playwright-timeout",
  },
  {
    name: "fixed Promise.resolve turn loop",
    source:
      "for (let turn = 0; turn < 8; turn += 1) await globalThis.Promise.resolve();",
    kind: "promise-turn-loop",
  },
  {
    name: "setImmediate correctness turn",
    source: "await new Promise((resolve) => setImmediate(resolve));",
    kind: "promise-turn-wait",
  },
];

for (const mutation of rejectedJavaScriptMutations) {
  assert.deepEqual(
    violationKinds(mutation.source),
    [mutation.kind],
    mutation.name,
  );
}

const acceptedJavaScriptFixtures = [
  {
    name: "line comment",
    source:
      "// await new Promise((resolve) => setTimeout(() => resolve(), 10));\nobserve();",
  },
  {
    name: "block comment",
    source:
      "/* await page.waitForTimeout(10); */\nawait lifecycleBarrier.promise;",
  },
  {
    name: "quoted source",
    source:
      'const mutation = "await new Promise((resolve) => setTimeout(resolve, 10))";',
  },
  {
    name: "regular-expression source",
    source: "const mutation = /new Promise (setTimeout(resolve, 10))/u;",
  },
  {
    name: "template raw text",
    source:
      "const mutation = `await new Promise((resolve) => setTimeout(resolve, 10))`;",
  },
  {
    name: "template interpolation without a delay",
    source: "const value = `prefix ${String(observed)} suffix`;",
  },
  {
    name: "similar property name",
    source: "await page.waitForTimeoutBudget(10);",
  },
  {
    name: "shadowed timer parameter",
    source:
      "async function test(setTimeout) { await new Promise((resolve) => setTimeout(resolve)); }",
  },
  {
    name: "destructured deterministic scheduler callback",
    source:
      "const { setTimeout: schedule } = fakeScheduler; await new Promise((resolve) => schedule(resolve));",
  },
  {
    name: "product timer",
    source: "const timer = window.setTimeout(expireLease, leaseTtlMs);",
  },
  {
    name: "non-Playwright helper declaration",
    source: "function waitForTimeout(milliseconds) { schedule(milliseconds); }",
  },
  {
    name: "deterministic fake clock",
    source: "scheduler.advanceBy(10);",
  },
];

for (const fixture of acceptedJavaScriptFixtures) {
  assert.deepEqual(violationKinds(fixture.source), [], fixture.name);
}

assert.deepEqual(
  violationKinds(
    "const pattern = /ignored \\/\\/ comment text/u; await page.waitForTimeout(10);",
  ),
  ["playwright-timeout"],
  "a regular expression cannot hide executable code that follows it",
);

const rejectedRustMutations = [
  [
    "Tokio sleep",
    "tokio::time::sleep(Duration::from_millis(10)).await;",
    "rust-sleep",
  ],
  [
    "standard sleep",
    "std::thread::sleep(Duration::from_millis(10));",
    "rust-sleep",
  ],
  [
    "imported thread sleep",
    "thread::sleep(Duration::from_millis(10));",
    "rust-sleep",
  ],
  ["standard yield spin", "std::thread::yield_now();", "rust-spin-wait"],
  ["Tokio yield spin", "tokio::task::yield_now().await;", "rust-spin-wait"],
  ["hint spin loop", "std::hint::spin_loop();", "rust-spin-wait"],
  [
    "imported Tokio sleep",
    "use tokio::time::sleep; sleep(Duration::from_millis(10)).await;",
    "rust-sleep",
  ],
  [
    "aliased Tokio sleep",
    "use tokio::time::sleep as nap; nap(Duration::from_millis(10)).await;",
    "rust-sleep",
  ],
  [
    "grouped aliased thread yield",
    "use std::thread::{sleep as nap, yield_now as yield_thread}; yield_thread();",
    "rust-spin-wait",
  ],
  [
    "module-aliased thread sleep",
    "use std::thread as worker; worker::sleep(Duration::from_millis(10));",
    "rust-sleep",
  ],
  [
    "imported core spin",
    "use core::hint::spin_loop as spin; spin();",
    "rust-spin-wait",
  ],
];

for (const [name, source, kind] of rejectedRustMutations) {
  assert.deepEqual(violationKinds(source, "rust"), [kind], name);
}

const acceptedRustFixtures = [
  [
    "line comment",
    "// std::thread::sleep(Duration::from_secs(1));\nobserve();",
  ],
  [
    "nested block comment",
    "/* outer /* tokio::time::sleep(delay).await; */ outer */\nobserve();",
  ],
  ["quoted source", 'let mutation = "std::thread::yield_now()";'],
  ["raw quoted source", 'let mutation = r#"tokio::task::yield_now().await"#;'],
  ["bounded watchdog", "tokio::time::timeout(deadline, task).await?;"],
  ["condition notification", "condition.notified().await;"],
  [
    "unrelated imported sleep name",
    "use crate::fixture::sleep; sleep(assertion_state);",
  ],
  [
    "similar function name",
    "tokio::time::sleep_until_observed(condition).await;",
  ],
];

for (const [name, source] of acceptedRustFixtures) {
  assert.deepEqual(violationKinds(source, "rust"), [], name);
}

assert.deepEqual(
  violationKinds(`
// suprnova-correctness-delay-allow: watchdog -- failure-only deadline for a spawned test process
await new Promise((resolve) => setTimeout(() => resolve(), 10));
`),
  [],
  "a narrow reasoned watchdog exception is accepted",
);

assert.deepEqual(
  violationKinds(`
function retry() {
  // suprnova-correctness-delay-allow: product-timer -- intentional retry cadence is observable production behavior
  return new Promise((resolve) => setTimeout(resolve, 10));
}
`),
  [],
  "an indented reasoned exception binds to the following syntax node",
);

for (const malformed of [
  { language: "javascript", source: "await new Promise((resolve) => {" },
  { language: "javascript", source: "const value = `unterminated ${work();" },
  { language: "rust", source: "use tokio::time::{sleep as nap;" },
  { language: "rust", source: 'let value = r#"unterminated;' },
]) {
  assert.deepEqual(
    violationKinds(malformed.source, malformed.language),
    ["parse-error"],
    `${malformed.language} parse errors fail closed`,
  );
}

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
assert.deepEqual(
  parseRustCandidates(repositoryRoot, [
    { file_path: "valid.rs", source: "fn valid() {}" },
  ]),
  [],
  "the Rust parser accepts a valid verification source",
);
assert.deepEqual(
  parseRustCandidates(repositoryRoot, [
    { file_path: "invalid.rs", source: "fn invalid(" },
  ]).map(({ file_path: filePath, kind }) => ({ filePath, kind })),
  [{ filePath: "invalid.rs", kind: "parse-error" }],
  "the parser-backed Rust validation fails closed on grammar errors",
);
const surfacePaths = iteration004VerificationSurfaces(repositoryRoot).map(
  ({ filePath }) => path.relative(repositoryRoot, filePath),
);
for (const required of [
  "tests/upload_file_provider.rs",
  "tests/upload_cleanup.rs",
  "tests/async_subscription.rs",
  "crates/suprnova-live-test-support/src/reference_host/mod.rs",
  "crates/suprnova-live-test-support/src/reference_host/uploads.rs",
  "browser/tests/upload-manager.test.ts",
  "browser/tests/async-feature.test.ts",
  "browser/e2e/async-lifecycle.spec.ts",
  "browser/e2e/iteration-004-integration.spec.ts",
  "browser/test-host/server.mjs",
  "browser/test-host/iteration-004.mjs",
]) {
  assert.ok(
    surfacePaths.includes(required),
    `missing owned verification surface: ${required}`,
  );
}
assert.equal(
  new Set(surfacePaths).size,
  surfacePaths.length,
  "the verification-surface manifest contains no duplicates",
);

assert.deepEqual(
  scanRepository(repositoryRoot),
  [],
  "the complete owned verification surface is free of correctness delays",
);

assert.deepEqual(
  violationKinds(`
// suprnova-correctness-delay-allow: anything -- this category is not approved
await new Promise((resolve) => setTimeout(resolve, 10));
`),
  ["invalid-allow", "promise-timeout"],
  "an unknown exception category fails closed",
);

assert.deepEqual(
  violationKinds(`
// suprnova-correctness-delay-allow: watchdog -- short
await new Promise((resolve) => setTimeout(resolve, 10));
`),
  ["invalid-allow", "promise-timeout"],
  "an unexplained exception fails closed",
);

assert.deepEqual(
  violationKinds(`
// suprnova-correctness-delay-allow: fake-clock -- deliberate scheduler advancement fixture
scheduler.advanceBy(10);
`),
  ["unused-allow"],
  "a stale exception fails closed",
);

assert.deepEqual(
  violationKinds(
    "const source = `ignored ${await new Promise((resolve) => setTimeout(resolve, 10))}`;",
  ),
  ["promise-timeout"],
  "executable template interpolation remains visible",
);

printf("correctness-delay scanner tests ok\n");

function printf(message) {
  process.stdout.write(message);
}
