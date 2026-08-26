import { createHash } from "node:crypto";
import { lstat, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const baselinePath = resolve(browserRoot, "benchmarks/baselines/browser-budget-v1.json");
const candidatePath = resolve(
  process.env["SUPRNOVA_LIVE_BROWSER_BUDGET_CANDIDATE"] ??
    resolve(browserRoot, "benchmarks/local/latest.json"),
);
const COMPATIBLE_CORE = ">=0.1.0 <0.2.0";
const ROLE_CEILINGS = new Map([
  ["core-esm", null],
  ["core-classic", null],
  ["stimulus-esm", 8 * 1024],
  ["stimulus-classic", 8 * 1024],
  ["uploads-esm", 20 * 1024],
  ["uploads-classic", 20 * 1024],
  ["async-esm", 16 * 1024],
  ["async-classic", 16 * 1024],
]);

export function evaluateArtifactBudgets(assets) {
  const byRole = new Map();
  const duplicateRoles = new Set();
  const unknownRoles = new Set();
  for (const asset of assets) {
    if (!ROLE_CEILINGS.has(asset.role)) {
      unknownRoles.add(String(asset.role));
      continue;
    }
    if (byRole.has(asset.role)) duplicateRoles.add(asset.role);
    else byRole.set(asset.role, asset);
  }

  const lines = [];
  const issues = [];
  for (const role of [...duplicateRoles].sort()) {
    issues.push(`artifact_budget:duplicate:${role}`);
  }
  for (const role of ROLE_CEILINGS.keys()) {
    if (!byRole.has(role)) issues.push(`artifact_budget:missing:${role}`);
  }
  for (const role of [...unknownRoles].sort()) issues.push(`artifact_budget:unknown:${role}`);
  for (const [role] of ROLE_CEILINGS) {
    const asset = byRole.get(role);
    if (asset !== undefined && asset.compatibleCore !== COMPATIBLE_CORE) {
      issues.push(`artifact_budget:compatible_core:${role}`);
    }
  }
  for (const [role, ceiling] of ROLE_CEILINGS) {
    const asset = byRole.get(role);
    const bytes = asset?.brotliBytes;
    lines.push(
      `artifact_budget role=${role} bytes=${bytes === undefined ? "missing" : String(bytes)} ceiling=${String(ceiling ?? "none")}`,
    );
    if (!Number.isSafeInteger(bytes) || bytes < 0) {
      if (asset !== undefined) issues.push(`artifact_budget:bytes:${role}`);
    } else if (ceiling !== null && bytes > ceiling) {
      issues.push(`artifact_budget:${role}:+${String(bytes - ceiling)}`);
    }
  }
  return Object.freeze({ lines: Object.freeze(lines), issues: Object.freeze(issues) });
}

export function evaluateBindingEvidence(
  baseline,
  candidate,
  runtimeArtifact,
  evaluate,
  release = false,
) {
  if (candidate === null) throw new Error("browser_budget_candidate_missing");
  if (Date.parse(candidate.recordedAt) <= Date.parse(baseline.recordedAt)) {
    throw new Error("browser_budget_candidate_stale");
  }
  if (
    candidate.artifact.sha256 !== runtimeArtifact.sha256 ||
    candidate.artifact.brotliBytes !== runtimeArtifact.brotliBytes
  ) {
    throw new Error("browser_budget_candidate_artifact_mismatch");
  }
  if (candidate.methodology.independentRuns < 3) {
    throw new Error("browser_budget_candidate_runs");
  }
  return evaluate(candidate, baseline, { release });
}

async function boundedBenchmarkJson(path, missingAllowed) {
  let metadata;
  try {
    metadata = await lstat(path);
  } catch (error) {
    if (missingAllowed && error instanceof Error && "code" in error && error.code === "ENOENT") {
      return null;
    }
    throw new Error("browser_budget_evidence_unreadable", { cause: error });
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 1_048_576) {
    throw new Error("browser_budget_evidence_unreadable");
  }
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    throw new Error("browser_budget_evidence_invalid");
  }
}

async function checkBudgets(release, binding) {
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
  const manifest = JSON.parse(
    await readFile(resolve(browserRoot, "dist/suprnova-live.assets.json"), "utf8"),
  );
  const measured = [];
  for (const asset of manifest.assets) {
    const content = await readFile(resolve(browserRoot, "dist", asset.file));
    measured.push({
      role: asset.role,
      file: asset.file,
      compatibleCore: asset.compatible_core,
      brotliBytes: brotliCompressSync(content, {
        params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
      }).byteLength,
    });
  }
  const artifactBudgets = evaluateArtifactBudgets(measured);
  process.stdout.write(`${artifactBudgets.lines.join("\n")}\n`);
  if (artifactBudgets.issues.length > 0) {
    throw new Error(`artifact_budget_failed:${artifactBudgets.issues.join(",")}`);
  }
  const runtimeAsset = measured.find(({ role }) => role === "core-esm");
  if (runtimeAsset === undefined) throw new Error("artifact_budget:missing:core-esm");
  const runtime = await readFile(resolve(browserRoot, "dist", runtimeAsset.file));
  const runtimeSha256 = createHash("sha256").update(runtime).digest("hex");
  const brotliBytes = runtimeAsset.brotliBytes;

  if (!binding) {
    console.log(
      `budget ok control_overhead=${controlOverhead} snapshot_overhead=${snapshotOverhead} core_brotli=${brotliBytes} browser_binding=skipped`,
    );
    return;
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
    await boundedBenchmarkJson(baselinePath, false),
  );
  const candidateValue = await boundedBenchmarkJson(candidatePath, true);
  const candidate =
    candidateValue === null ? null : schema.validateBrowserBudgetResult(candidateValue);
  const evaluation = evaluateBindingEvidence(
    baseline,
    candidate,
    { brotliBytes, sha256: runtimeSha256 },
    (candidate, baseline, options) => schema.evaluateBrowserBudget(candidate, baseline, options),
    release,
  );
  if (evaluation.status === "failed") {
    throw new Error(`browser budget failed: ${evaluation.codes.join(",")}`);
  }
  if (evaluation.status === "unqualified") {
    process.stdout.write(
      `browser budget unqualified classification=${candidate.classification} codes=${evaluation.codes.join(",")}\n`,
    );
    process.exitCode = 2;
  }

  console.log(
    `budget ok control_overhead=${controlOverhead} snapshot_overhead=${snapshotOverhead} core_brotli=${brotliBytes} browser_candidate=${candidate.classification} baseline_artifact=prior`,
  );
}

const invokedPath = process.argv[1] === undefined ? "" : resolve(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) {
  const arguments_ = process.argv.slice(2);
  const release = arguments_.includes("--release");
  const binding = release || arguments_.includes("--binding");
  if (arguments_.some((argument) => argument !== "--release" && argument !== "--binding")) {
    throw new Error("usage: node scripts/check-budget.mjs [--binding] [--release]");
  }
  await checkBudgets(release, binding);
}
