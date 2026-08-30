#!/usr/bin/env node

import assert from "node:assert/strict";

import { scanSource } from "../scripts/check-correctness-delays.mjs";

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
