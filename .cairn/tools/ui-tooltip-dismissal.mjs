#!/usr/bin/env node
// Mechanism for OVL-002 and OVL-008: the shipped tooltip in each qualified
// engine. The browser cases carry their requirement in the title, and this
// runs them once and reports each requirement from the cases that name it.
//
//   node .cairn/tools/ui-tooltip-dismissal.mjs
//
// A requirement passes when at least one case names it and none of those
// cases failed. A run that produces no report fails both requirements.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BROWSER = "crates/suprnova-live/browser";
const IDS = ["OVL-002", "OVL-008"];
const cases = new Map(IDS.map((id) => [id, { failed: [], passed: 0 }]));

const scratch = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "ui-tooltip-dismissal-"));
let broken = null;
try {
  const output = join(scratch, "report.json");
  process.stdout.write("run: tooltip component cases\n");
  const result = spawnSync(
    "npx",
    [
      "playwright",
      "test",
      "--config",
      "playwright.components.config.ts",
      "e2e/components/tooltip.spec.ts",
      "--reporter=json",
    ],
    {
      cwd: BROWSER,
      encoding: "utf8",
      env: { ...process.env, PLAYWRIGHT_JSON_OUTPUT_NAME: output },
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  process.stdout.write(`  tooltip component cases: exit ${String(result.status)}\n`);
  let report;
  try {
    report = JSON.parse(readFileSync(output, "utf8"));
  } catch {
    broken = `the run produced no report (exit ${String(result.status)})`;
  }
  const walk = (suite, path) => {
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests ?? []) {
        const title = [...path, spec.title].join(" > ");
        const ok = test.status === "expected" || test.status === "flaky";
        for (const id of IDS) {
          if (!new RegExp(`(^|[^A-Z0-9-])${id}([^0-9]|$)`).test(title)) continue;
          const entry = cases.get(id);
          if (ok) entry.passed += 1;
          else entry.failed.push(`${title} (${test.projectName ?? ""})`.trim());
        }
      }
    }
    for (const child of suite.suites ?? []) walk(child, [...path, child.title]);
  };
  for (const suite of report?.suites ?? []) walk(suite, [suite.title]);
} finally {
  rmSync(scratch, { force: true, recursive: true });
}

let failed = false;
for (const id of IDS) {
  const entry = cases.get(id);
  const why =
    broken ??
    (entry.failed.length > 0
      ? entry.failed.join("; ")
      : entry.passed === 0
        ? "no case names it"
        : null);
  if (why === null) {
    process.stdout.write(`cairn: ${id}: pass\n`);
  } else {
    failed = true;
    process.stdout.write(`cairn: ${id}: fail\n  reason: ${why}\n`);
  }
}
process.exit(failed ? 1 : 0);
