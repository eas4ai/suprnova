#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  parseRustCandidates,
  resolveCargoTargetDirectory,
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

const scriptOpen = String.fromCharCode(60, 115, 99, 114, 105, 112, 116);
const scriptClose = String.fromCharCode(
  60,
  47,
  115,
  99,
  114,
  105,
  112,
  116,
  62,
);

const manifestRoot = fs.mkdtempSync(
  path.join(os.tmpdir(), "suprnova-live-verification-surfaces-"),
);
try {
  const expectedNestedSurfaces = [
    "tests/nested/upload/regression.rs",
    "crates/suprnova-live-test-support/src/bin/nested/harness.rs",
    "crates/suprnova-live-test-support/tests/nested/host.rs",
    "browser/tests/nested/unit.test.ts",
    "browser/e2e/nested/lifecycle.spec.ts",
    "browser/test-host/nested/scenario.mjs",
    "src/nested/inline_tests.rs",
    "fuzz/fuzz_targets/nested/upload.rs",
    "benches/nested/async_budget.rs",
  ];
  for (const relative of expectedNestedSurfaces) {
    const absolute = path.join(manifestRoot, relative);
    fs.mkdirSync(path.dirname(absolute), { recursive: true });
    fs.writeFileSync(absolute, "// verification fixture\n", "utf8");
  }
  const generatedSurface = "tests/fixtures/compile/target/generated.rs";
  const generatedAbsolute = path.join(manifestRoot, generatedSurface);
  fs.mkdirSync(path.dirname(generatedAbsolute), { recursive: true });
  fs.writeFileSync(generatedAbsolute, "// generated fixture\n", "utf8");
  const discovered = new Set(
    iteration004VerificationSurfaces(manifestRoot).map(({ filePath }) =>
      path.relative(manifestRoot, filePath),
    ),
  );
  for (const relative of expectedNestedSurfaces) {
    assert.equal(
      discovered.has(relative),
      true,
      `recursive verification ownership must discover ${relative}`,
    );
  }
  assert.equal(
    discovered.has(generatedSurface),
    false,
    "generated build output is not verification-owned source",
  );
} finally {
  fs.rmSync(manifestRoot, { force: true, recursive: true });
}

const rejectedJavaScriptMutations = [
  {
    name: "direct timeout-resolved promise",
    source: "await new Promise((resolve) => setTimeout(resolve, 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "callback timeout-resolved promise",
    source: "await new Promise((resolve) => setTimeout(() => resolve(), 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "window-qualified timeout-resolved promise",
    source:
      "await new Promise((resolve) => window.setTimeout(() => resolve(), 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "Playwright wall-clock wait",
    source: "await page.waitForTimeout(10);",
    kind: "delay-primitive-reference",
  },
  {
    name: "global Promise and optional qualified timer",
    source:
      "await new globalThis.Promise((resolve) => globalThis?.setTimeout?.(resolve, 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "aliased timer",
    source:
      "const later = window.setTimeout; await new Promise((resolve) => later(resolve, 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "destructured timer",
    source:
      "const { setTimeout: later } = globalThis; await new Promise((resolve) => later(resolve, 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "computed destructured timer",
    source:
      'const { ["setTimeout"]: later } = globalThis; await new Promise((resolve) => later(resolve, 10));',
    kind: "delay-primitive-reference",
  },
  {
    name: "destructuring assignment timer",
    source:
      "let later; ({ setTimeout: later } = globalThis); await new Promise((resolve) => later(resolve, 10));",
    kind: "delay-primitive-reference",
  },
  {
    name: "computed destructuring assignment Playwright wait",
    source: 'let wait; ({ ["waitForTimeout"]: wait } = page); await wait(10);',
    kind: "delay-primitive-reference",
  },
  {
    name: "computed qualified timer alias",
    source:
      'const later = globalThis["setTimeout"]; await new Promise((resolve) => later(resolve, 10));',
    kind: "delay-primitive-reference",
  },
  {
    name: "timers promises import alias",
    source:
      'import { setTimeout as delay } from "node:timers/promises"; await delay(10);',
    kind: "delay-module-reference",
  },
  {
    name: "aliased Playwright wait",
    source: "const wait = page.waitForTimeout; await wait(10);",
    kind: "delay-primitive-reference",
  },
  {
    name: "destructured Playwright wait",
    source: "const { waitForTimeout: wait } = page; await wait(10);",
    kind: "delay-primitive-reference",
  },
  {
    name: "optional Playwright wait",
    source: "await page?.waitForTimeout?.(10);",
    kind: "delay-primitive-reference",
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
    kind: "delay-primitive-reference",
  },
  {
    name: "timer reference is rejected before alias flow matters",
    source: "const later = globalThis.setTimeout; observe(later);",
    kind: "delay-primitive-reference",
  },
  {
    name: "bound timer wrapper",
    source:
      "const later = globalThis.setTimeout.bind(globalThis); observe(later);",
    kind: "delay-primitive-reference",
  },
  {
    name: "nested global-object timer path",
    source: "const later = globalThis.window?.setTimeout; observe(later);",
    kind: "delay-primitive-reference",
  },
  {
    name: "Reflect timer lookup",
    source: 'const later = Reflect.get(window, "setTimeout"); observe(later);',
    kind: "delay-primitive-reference",
  },
  {
    name: "timer module namespace import",
    source: 'import * as timers from "node:timers"; observe(timers);',
    kind: "delay-module-reference",
  },
  {
    name: "timer module default import",
    source: 'import timers from "timers"; observe(timers);',
    kind: "delay-module-reference",
  },
  {
    name: "timer module side-effect import",
    source: 'import "node:timers/promises";',
    kind: "delay-module-reference",
  },
  {
    name: "timer module require",
    source: 'const timers = require("node:timers"); observe(timers);',
    kind: "delay-module-reference",
  },
  {
    name: "timer module dynamic import",
    source:
      'const timers = await import("node:timers/promises"); observe(timers);',
    kind: "delay-module-reference",
  },
  {
    name: "shadowed timer remains conservatively forbidden",
    source:
      "async function test(setTimeout) { await new Promise((resolve) => setTimeout(resolve)); }",
    kind: "delay-primitive-reference",
  },
  {
    name: "locally declared Playwright-shaped delay remains forbidden",
    source: "function waitForTimeout(milliseconds) { observe(milliseconds); }",
    kind: "delay-primitive-reference",
  },
  {
    name: "executable inline module timer",
    source: `const html = \`${scriptOpen} type="module">await new Promise((resolve) => setTimeout(resolve, 10));${scriptClose}\`;`,
    kind: "delay-primitive-reference",
  },
  {
    name: "uppercase executable inline module timer",
    source: `const html = \`${scriptOpen.toUpperCase()} TYPE="MODULE">setTimeout(run, 10);${scriptClose.toUpperCase()}\`;`,
    kind: "delay-primitive-reference",
  },
  {
    name: "static concatenated inline timer body",
    source:
      'const html = "<scr" + "ipt type=module>" + "setTimeout(run, 10);" + "</scr" + "ipt>";',
    kind: "delay-primitive-reference",
  },
  {
    name: "data-src is not an external script exemption",
    source: `const html = \`${scriptOpen} data-src="/scenario.js">setTimeout(run, 10);${scriptClose}\`;`,
    kind: "delay-primitive-reference",
  },
  {
    name: "data-type is not an inert script exemption",
    source: `const html = \`${scriptOpen} data-type="application/json">setTimeout(run, 10);${scriptClose}\`;`,
    kind: "delay-primitive-reference",
  },
  {
    name: "quoted data value cannot forge a src attribute",
    source: `const html = \`${scriptOpen} data-note=" src='/scenario.js'">setTimeout(run, 10);${scriptClose}\`;`,
    kind: "delay-primitive-reference",
  },
  {
    name: "quoted data value cannot forge an inert type attribute",
    source: `const html = \`${scriptOpen} data-note=" type='application/json'">setTimeout(run, 10);${scriptClose}\`;`,
    kind: "delay-primitive-reference",
  },
];

for (const mutation of rejectedJavaScriptMutations) {
  assert.deepEqual(
    violationKinds(mutation.source),
    [mutation.kind],
    mutation.name,
  );
}

assert.deepEqual(
  violationKinds(
    `const scriptType = configuredType(); const html = \`${scriptOpen} type="\${scriptType}">setTimeout(run, 10);${scriptClose}\`;`,
  ).sort(),
  ["delay-primitive-reference", "inline-script-assembly"],
  "an interpolated script type is executable and fails closed as dynamic assembly",
);

assert.deepEqual(
  violationKinds(
    'const body = renderBody(); const html = "<scr" + "ipt>" + body + "</scr" + "ipt>";',
  ),
  ["inline-script-assembly"],
  "dynamic inline script assembly fails closed",
);

assert.deepEqual(
  violationKinds(
    'const suffix = selectedTagSuffix(); const html = "<scr" + suffix + ">setTimeout(run, 10);</scr" + "ipt>";',
  ),
  ["inline-script-assembly"],
  "a dynamically completed script tag name fails closed",
);

assert.deepEqual(
  violationKinds(
    "const tag = selectedTag(); const html = `<${tag}>setTimeout(run, 10);</${tag}>`;",
  ),
  ["inline-script-assembly"],
  "an interpolated tag name fails closed",
);

assert.deepEqual(
  violationKinds(
    'const attrs = selectedAttributes(); const html = `<script ${attrs} src="/safe.js"></script>`;',
  ),
  ["inline-script-assembly"],
  "a static external source cannot excuse dynamic script attributes",
);

assert.deepEqual(
  violationKinds(
    'const attrs = selectedAttributes(); const html = `<script ${attrs} src="/safe.js">setTimeout(run, 10);</script>`;',
  ).sort(),
  ["delay-primitive-reference", "inline-script-assembly"],
  "dynamic attributes that can close and reopen a script fail before external-source exemption",
);

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
    name: "ordinary element with a dynamic attribute value",
    source: 'const html = `<div data-kind="${kind}">ordinary content</div>`;',
  },
  {
    name: "provably inert inline JSON script",
    source: `const html = \`${scriptOpen} type="application/json">{"ready":true}${scriptClose}\`;`,
  },
  {
    name: "static external script",
    source: `const html = \`${scriptOpen} type="module" src="/scenario.js">${scriptClose}\`;`,
  },
  {
    name: "similar property name",
    source: "await page.waitForTimeoutBudget(10);",
  },
  {
    name: "benign interface and object property keys",
    source: `
interface DelayMetadata { setTimeout: string; waitForTimeout: boolean }
const metadata: DelayMetadata = { setTimeout: "documented", waitForTimeout: false };
const computed = { ["setTimeout"]: "documented", ["waitForTimeout"]: false };
observe(metadata, computed);
`,
  },
  {
    name: "deterministic fake clock",
    source: "scheduler.advanceBy(10);",
  },
  {
    name: "reasoned fake scheduler primitive",
    source: `
// suprnova-correctness-delay-allow: fake-clock -- deterministic scheduler installation for controlled virtual time
const { setTimeout: schedule } = fakeScheduler;
observe(schedule);
`,
  },
  {
    name: "reasoned product timer",
    source: `
// suprnova-correctness-delay-allow: product-timer -- lease expiry is observable behavior rather than test synchronization
const timer = window.setTimeout(expireLease, leaseTtlMs);
observe(timer);
`,
  },
  {
    name: "reasoned Playwright watchdog configuration",
    source: `
// suprnova-correctness-delay-allow: watchdog -- suite deadline is a failure bound rather than correctness synchronization
test.setTimeout(60_000);
`,
  },
];

for (const fixture of acceptedJavaScriptFixtures) {
  assert.deepEqual(violationKinds(fixture.source), [], fixture.name);
}

assert.deepEqual(
  violationKinds(
    "const pattern = /ignored \\/\\/ comment text/u; await page.waitForTimeout(10);",
  ),
  ["delay-primitive-reference"],
  "a regular expression cannot hide executable code that follows it",
);

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
const workspaceRoot = path.resolve(repositoryRoot, "..", "..");
const expectedTargetDirectory = process.env.CARGO_TARGET_DIR
  ? path.resolve(workspaceRoot, process.env.CARGO_TARGET_DIR)
  : path.join(workspaceRoot, "target");
assert.equal(
  resolveCargoTargetDirectory(repositoryRoot),
  expectedTargetDirectory,
  "the integrated scanner resolves the parent Cargo workspace target directory",
);
assert.notEqual(
  resolveCargoTargetDirectory(repositoryRoot),
  path.join(repositoryRoot, "target"),
  "the integrated scanner must not fall back to a nested Live target directory",
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
  "browser/test-host/stimulus-scenario.mjs",
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
  violationKinds(`
// suprnova-correctness-delay-allow: anything -- this category is not approved
await new Promise((resolve) => setTimeout(resolve, 10));
`),
  ["invalid-allow", "delay-primitive-reference"],
  "an unknown exception category fails closed",
);

assert.deepEqual(
  violationKinds(`
// suprnova-correctness-delay-allow: watchdog -- short
await new Promise((resolve) => setTimeout(resolve, 10));
`),
  ["invalid-allow", "delay-primitive-reference"],
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
  ["delay-primitive-reference"],
  "executable template interpolation remains visible",
);

printf("correctness-delay scanner fixtures ok\n");

assert.deepEqual(
  scanRepository(repositoryRoot),
  [],
  "the complete owned verification surface is free of correctness delays",
);

printf("correctness-delay repository scan ok\n");

function printf(message) {
  process.stdout.write(message);
}
