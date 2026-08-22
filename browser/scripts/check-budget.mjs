import { createHash } from "node:crypto";
import { lstat, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const baselinePath = resolve(browserRoot, "benchmarks/baselines/browser-budget-v1.json");
const release = process.argv.slice(2).includes("--release");
if (process.argv.slice(2).some((argument) => argument !== "--release")) {
  throw new Error("usage: node scripts/check-budget.mjs [--release]");
}

const fixtureUrl = new URL("../../fixtures/v1/snapshot-success.json", import.meta.url);
const fixtures = JSON.parse(await readFile(fixtureUrl, "utf8"));
const instance = fixtures.cases.find((fixture) => fixture.id === "instance-v1");
if (!instance) throw new Error("missing instance-v1 budget fixture");

const snapshot = JSON.stringify(instance.encoded);
const stateBytes = JSON.stringify(instance.encoded.body.state).length;
const memoBytes = JSON.stringify(instance.encoded.body.memo).length;
const snapshotOverhead = snapshot.length - stateBytes - memoBytes;
if (snapshotOverhead > 768) {
  throw new Error(`snapshot overhead ${snapshotOverhead} exceeds 768 bytes`);
}

const html = "h".repeat(8 * 1024);
const payload = "s".repeat(16 * 1024);
const response = JSON.stringify({
  accepted_revision: "8",
  correlation_id: "EBESExQVFhcYGRobHB0eHw",
  effects: [],
  events: [],
  extensions: {},
  outcome: "accepted",
  protocol_version: 1,
  render: { html, kind: "html" },
  snapshot: { body: { payload }, signature: "A".repeat(43) },
  validation: {},
});
const controlOverhead = response.length - html.length - payload.length;
if (controlOverhead > 1024) {
  throw new Error(`control overhead ${controlOverhead} exceeds 1024 bytes`);
}

await buildRuntimeAssets();
const runtime = await readFile(resolve(browserRoot, "dist/suprnova-live.esm.js"));
const runtimeSha256 = createHash("sha256").update(runtime).digest("hex");
const brotliBytes = brotliCompressSync(runtime, {
  params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
}).byteLength;
if (brotliBytes > 45 * 1024) {
  throw new Error(`core runtime ${brotliBytes} Brotli bytes exceeds 46080 bytes`);
}

const baselineMetadata = await lstat(baselinePath);
if (
  !baselineMetadata.isFile() ||
  baselineMetadata.isSymbolicLink() ||
  baselineMetadata.size > 1_048_576
) {
  throw new Error("browser budget baseline invalid");
}
const compiled = await build({
  absWorkingDir: browserRoot,
  bundle: true,
  entryPoints: [resolve(browserRoot, "benchmarks/schema.ts")],
  format: "esm",
  legalComments: "none",
  minify: true,
  platform: "node",
  target: "node20",
  write: false,
});
const schemaOutput = compiled.outputFiles[0];
if (schemaOutput === undefined) throw new Error("browser budget schema build failed");
const schema = await import(
  `data:text/javascript;base64,${Buffer.from(schemaOutput.contents).toString("base64")}`
);
const baseline = schema.validateBrowserBudgetResult(
  JSON.parse(await readFile(baselinePath, "utf8")),
);
if (baseline.artifact.sha256 !== runtimeSha256 || baseline.artifact.brotliBytes !== brotliBytes) {
  throw new Error("browser budget baseline artifact is stale");
}
const evaluation = schema.evaluateBrowserBudget(baseline, baseline, { release });
if (evaluation.status === "failed") {
  throw new Error(`browser budget failed: ${evaluation.codes.join(",")}`);
}
if (evaluation.status === "unqualified") {
  process.stdout.write(
    `browser budget unqualified classification=${baseline.classification} codes=${evaluation.codes.join(",")}\n`,
  );
  process.exitCode = 2;
}

console.log(
  `budget ok control_overhead=${controlOverhead} snapshot_overhead=${snapshotOverhead} core_brotli=${brotliBytes} browser_baseline=${baseline.classification}`,
);
