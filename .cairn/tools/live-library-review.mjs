#!/usr/bin/env node
// Mechanism for live-library-review-remediation: every requirement the
// commitment names is proved by test cases that carry its identifier, and
// this runs each group of cases once and reports each requirement from the
// cases that name it.
//
//   node .cairn/tools/live-library-review.mjs
//
// A requirement passes when at least one case names it and none of those
// cases failed. A group that could not run fails every requirement it owns,
// with the group named. Each run is labeled in the output.
//
// Groups:
//   component browser cases  playwright.components.config.ts, Chromium,
//                            Firefox and WebKit, titles "<ID>: ..."
//   dogfood form cases       playwright.dogfood.config.ts against the dogfood
//                            application host, titles "<ID>: ..." or
//                            "<ID> and <ID>: ..."
//   runtime unit cases       vitest, titles "<ID>: ..."
//   Rust cases               one nextest run per requirement and test
//                            target, over tests whose names start with the
//                            identifier in snake case (live_031_...)
//   manual text              LIVE-035's second obligation: manual/live.md
//                            says that a key `live_key` refuses fails the
//                            island's render, and names `live_key_digest`
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BROWSER = "crates/suprnova-live/browser";
const CARGO_ENV = { CARGO_INCREMENTAL: "0", CARGO_BUILD_JOBS: "12", RUST_TEST_THREADS: "12" };
const REQUIREMENTS = [
  "FORM-008", "FORM-009", "FORM-010", "FORM-011", "FORM-012", "FDB-007", "NAV-007", "OVL-007",
  "DATA-006", "UI-020", "UI-021", "UI-022", "UI-023", "UI-024", "LIVE-031", "LIVE-032",
  "LIVE-033", "LIVE-034", "LIVE-035", "LIVE-036",
];
// Rust cases: for each identifier, the package and test targets whose tests
// carry it, one nextest run per entry.
const RUST = {
  "DATA-006": [["-p", "suprnova-live", "--test", "view_charts"]],
  "LIVE-031": [["-p", "suprnova", "--test", "live_dogfood_forms"]],
  "LIVE-033": [["-p", "suprnova-live", "--test", "checker_regressions", "--test", "view_live_key"]],
  "LIVE-034": [["-p", "suprnova-live", "--test", "checker_regressions"]],
  "LIVE-035": [["-p", "suprnova-live", "--test", "view_live_key"]],
  "LIVE-036": [["-p", "suprnova-live", "--test", "checker_regressions"]],
  "FORM-009": [["-p", "app", "--test", "live_dogfood"]],
  "UI-021": [["-p", "suprnova", "--test", "live_assets"]],
  "UI-022": [["-p", "suprnova-cli", "--test", "live_add"]],
  "UI-023": [["-p", "suprnova-cli", "--test", "live_add"]],
};

const cases = new Map(REQUIREMENTS.map((id) => [id, { passed: 0, failed: [] }]));
const record = (title, ok, group) => {
  for (const id of REQUIREMENTS) {
    const snake = id.toLowerCase().replace("-", "_");
    const named = new RegExp(`(^|[^A-Z0-9-])${id}([^0-9]|$)`).test(title) || title.includes(`${snake}_`);
    if (!named) continue;
    const entry = cases.get(id);
    if (ok) entry.passed += 1;
    else entry.failed.push(`${group}: ${title}`);
  }
};
const broken = new Map();
const run = (label, command, args, options = {}) => {
  process.stdout.write(`run: ${label}\n`);
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    ...options,
    env: { ...process.env, ...(options.env ?? {}) },
  });
  process.stdout.write(`  ${label}: exit ${String(result.status)}\n`);
  return result;
};

// Playwright and vitest groups report each case through their JSON reporters.
const scratch = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "live-library-review-"));
try {
  const playwright = (label, config, owns) => {
    const output = join(scratch, `${label.replace(/\W+/g, "-")}.json`);
    const result = run(label, "npx", ["playwright", "test", "--config", config, "--reporter=json"], {
      cwd: BROWSER,
      env: { PLAYWRIGHT_JSON_OUTPUT_NAME: output },
    });
    let report;
    try {
      report = JSON.parse(readFileSync(output, "utf8"));
    } catch {
      for (const id of owns) broken.set(id, `${label} produced no report (exit ${String(result.status)})`);
      return;
    }
    const walk = (suite, path) => {
      for (const spec of suite.specs ?? []) {
        for (const test of spec.tests ?? []) {
          const ok = test.status === "expected" || test.status === "flaky";
          // A describe block may carry the identifier, so the case is
          // named by its whole title path.
          record([...path, spec.title].join(" > "), ok, `${label} ${test.projectName ?? ""}`.trim());
        }
      }
      for (const child of suite.suites ?? []) walk(child, [...path, child.title]);
    };
    for (const suite of report.suites ?? []) walk(suite, [suite.title]);
  };
  playwright("component browser cases", "playwright.components.config.ts", [
    "FORM-008", "FORM-011", "FORM-012", "FDB-007", "NAV-007", "OVL-007", "UI-020", "UI-024",
  ]);
  playwright("dogfood form cases", "playwright.dogfood.config.ts", ["FORM-009", "FORM-010", "LIVE-032"]);

  const vitestOutput = join(scratch, "vitest.json");
  const vitest = run("runtime unit cases", "npx", ["vitest", "run", "--reporter=json", `--outputFile=${vitestOutput}`], {
    cwd: BROWSER,
  });
  try {
    const report = JSON.parse(readFileSync(vitestOutput, "utf8"));
    for (const file of report.testResults ?? []) {
      for (const test of file.assertionResults ?? []) record(test.title, test.status === "passed", "runtime unit cases");
    }
  } catch {
    broken.set("LIVE-032", `runtime unit cases produced no report (exit ${String(vitest.status)})`);
  }

  process.stdout.write("run: manual text\n");
  const manual = readFileSync("manual/live.md", "utf8");
  record(
    "LIVE-035: manual/live.md says a key live_key refuses fails the island's render and names live_key_digest",
    manual.includes("`live_key` fails the island's render") && manual.includes("`live_key_digest`"),
    "manual text",
  );

  for (const [id, targets] of Object.entries(RUST)) {
    const filter = `test(/(^|::)${id.toLowerCase().replace("-", "_")}_/)`;
    targets.forEach((target, index) => {
      const label = `Rust cases for ${id}, run ${String(index + 1)} of ${String(targets.length)} (${target.join(" ")})`;
      const result = run(label, "cargo", ["nextest", "run", ...target, "-E", filter, "--no-tests=fail"], {
        env: CARGO_ENV,
      });
      const summary = /(\d+) tests? run: (\d+) passed/.exec(`${result.stdout}${result.stderr}`);
      if (result.status === 0 && summary && Number(summary[2]) > 0) {
        cases.get(id).passed += Number(summary[2]);
      } else {
        cases.get(id).failed.push(`${label} (exit ${String(result.status)})`);
        process.stdout.write(`${result.stdout}${result.stderr}`.split("\n").slice(-40).join("\n"));
      }
    });
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

let failed = false;
for (const id of REQUIREMENTS) {
  const entry = cases.get(id);
  const why = broken.get(id) ?? (entry.failed.length > 0 ? entry.failed.join("; ") : entry.passed === 0 ? "no case names it" : null);
  if (why === null) {
    process.stdout.write(`cairn: ${id}: pass\n`);
  } else {
    failed = true;
    process.stdout.write(`cairn: ${id}: fail\n  reason: ${why}\n`);
  }
}
process.exit(failed ? 1 : 0);
