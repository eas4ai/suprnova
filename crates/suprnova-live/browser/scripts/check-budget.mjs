import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { lstat, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const baselinePath = resolve(browserRoot, "benchmarks/baselines/browser-budget-v1.json");
const artifactSizeBaselinePath = resolve(browserRoot, "benchmarks/baselines/artifact-size-v1.json");
const artifactSizeBaselineRepositoryPath = "browser/benchmarks/baselines/artifact-size-v1.json";
const MAX_INVALID_BASELINE_COMMITS = 8;
const MAX_PROVENANCE_COMMITS = 256;
const MAX_REPOSITORY_PATH_BYTES = 1_024;
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
  ["async-esm", null],
  ["async-classic", null],
]);
const ASYNC_ARTIFACTS = new Map([
  ["async-esm", "suprnova-live.async.esm.js"],
  ["async-classic", "suprnova-live.async.classic.js"],
]);
const TASK6_REVIEW = Object.freeze({
  decision: "iteration-004-task-6",
  rationale:
    "Last reviewed complete Task 6 deterministic production artifacts before polling implementation.",
  recordedAt: "2026-08-26T06:27:18-04:00",
  sourceCommit: "499eda2287f17d6a46c9b8c306df5791b1f671d8",
});
const TASK6_ROLES = Object.freeze({
  "async-classic": Object.freeze({
    artifact: "suprnova-live.async.classic.js",
    brotliBytes: 14_155,
  }),
  "async-esm": Object.freeze({
    artifact: "suprnova-live.async.esm.js",
    brotliBytes: 16_356,
  }),
});

function exactKeys(value, expected) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

export function validateArtifactSizeBaseline(value) {
  try {
    if (
      !exactKeys(value, [
        "history",
        "maximumUnreviewedIncreaseBasisPoints",
        "methodology",
        "schemaVersion",
      ]) ||
      value.schemaVersion !== 2 ||
      value.maximumUnreviewedIncreaseBasisPoints !== 1_500 ||
      !exactKeys(value.methodology, ["buildCommand", "compression", "deterministic"]) ||
      value.methodology.buildCommand !== "npm run build" ||
      value.methodology.compression !== "brotli-quality-11" ||
      value.methodology.deterministic !== true ||
      !Array.isArray(value.history) ||
      value.history.length < 1 ||
      value.history.length > 64
    ) {
      throw new Error("artifact_size_baseline_invalid");
    }
    const decisions = new Set();
    let priorRecordedAt = Number.NEGATIVE_INFINITY;
    for (const [index, entry] of value.history.entries()) {
      if (!exactKeys(entry, ["review", "roles"])) {
        throw new Error("artifact_size_baseline_invalid");
      }
      const { review, roles } = entry;
      const anchorReview =
        index === 0 && exactKeys(review, ["decision", "rationale", "recordedAt", "sourceCommit"]);
      const provenanceReview =
        index > 0 &&
        exactKeys(review, [
          "decision",
          "rationale",
          "recordedAt",
          "sourceCommit",
          "sourceDecision",
          "sourceDecisionPath",
        ]);
      const recordedAt = Date.parse(review.recordedAt);
      if (
        (!anchorReview && !provenanceReview) ||
        !/^[a-z0-9][a-z0-9._-]{2,127}$/u.test(review.decision) ||
        decisions.has(review.decision) ||
        typeof review.rationale !== "string" ||
        review.rationale.length < 20 ||
        !Number.isFinite(recordedAt) ||
        recordedAt <= priorRecordedAt ||
        !/^[0-9a-f]{40}$/u.test(review.sourceCommit) ||
        (provenanceReview && review.sourceDecision !== review.decision) ||
        (provenanceReview &&
          !/^docs\/specs\/suprnova-live\/[a-z0-9][a-z0-9._/-]*\.md$/u.test(
            review.sourceDecisionPath,
          )) ||
        !exactKeys(roles, ["async-classic", "async-esm"])
      ) {
        throw new Error("artifact_size_baseline_invalid");
      }
      decisions.add(review.decision);
      priorRecordedAt = recordedAt;
      for (const [role, artifact] of ASYNC_ARTIFACTS) {
        const record = roles[role];
        const expectedKeys =
          index === 0 ? ["artifact", "brotliBytes"] : ["artifact", "brotliBytes", "sha256"];
        if (
          !exactKeys(record, expectedKeys) ||
          record.artifact !== artifact ||
          !Number.isSafeInteger(record.brotliBytes) ||
          record.brotliBytes <= 0 ||
          (index > 0 && !/^[0-9a-f]{64}$/u.test(record.sha256))
        ) {
          throw new Error("artifact_size_baseline_invalid");
        }
      }
    }
    const initial = value.history[0];
    if (
      Object.keys(TASK6_REVIEW).some((key) => initial.review[key] !== TASK6_REVIEW[key]) ||
      [...ASYNC_ARTIFACTS.keys()].some(
        (role) =>
          initial.roles[role].artifact !== TASK6_ROLES[role].artifact ||
          initial.roles[role].brotliBytes !== TASK6_ROLES[role].brotliBytes,
      )
    ) {
      throw new Error("artifact_size_baseline_invalid");
    }
    return Object.freeze(value);
  } catch {
    throw new Error("artifact_size_baseline_invalid");
  }
}

function git(repositoryRoot, arguments_) {
  try {
    return execFileSync("git", ["-C", repositoryRoot, ...arguments_], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch (error) {
    throw new Error("artifact_size_baseline_provenance_invalid", { cause: error });
  }
}

function committedFile(repositoryRoot, commit, repositoryPath) {
  git(repositoryRoot, ["cat-file", "-e", `${commit}^{commit}`]);
  const matches = git(repositoryRoot, [
    "ls-tree",
    "-r",
    "--full-tree",
    "--name-only",
    commit,
    "--",
    repositoryPath,
  ]);
  if (matches === "") return null;
  if (matches !== repositoryPath) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return git(repositoryRoot, ["show", `${commit}:${repositoryPath}`]);
}

function committedBaseline(repositoryRoot, commit, repositoryPath) {
  const source = committedFile(repositoryRoot, commit, repositoryPath);
  if (source === null) return null;
  try {
    return validateArtifactSizeBaseline(JSON.parse(source));
  } catch (error) {
    throw new Error("artifact_size_baseline_provenance_invalid", { cause: error });
  }
}

function sameEntry(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function boundedRepositoryPath(value) {
  if (
    typeof value !== "string" ||
    value.length < 1 ||
    Buffer.byteLength(value, "utf8") > MAX_REPOSITORY_PATH_BYTES ||
    value.startsWith("/") ||
    value.includes("\\") ||
    [...value].some((character) => {
      const codePoint = character.codePointAt(0);
      return codePoint !== undefined && (codePoint <= 31 || codePoint === 127);
    }) ||
    value.split("/").some((segment) => segment === "" || segment === "." || segment === "..")
  ) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return value;
}

function provenanceRepository(repositoryRoot) {
  const topLevel = git(repositoryRoot, ["rev-parse", "--show-toplevel"]);
  const rawPrefix = git(repositoryRoot, ["rev-parse", "--show-prefix"]);
  const prefix = rawPrefix.endsWith("/") ? rawPrefix.slice(0, -1) : rawPrefix;
  if (prefix !== "") boundedRepositoryPath(prefix);
  return Object.freeze({ prefix, topLevel });
}

function repositoryPathCandidates(prefix, recordedPath) {
  const legacyPath = boundedRepositoryPath(recordedPath);
  const candidates = [];
  if (prefix !== "") candidates.push(boundedRepositoryPath(`${prefix}/${legacyPath}`));
  candidates.push(legacyPath);
  return Object.freeze([...new Set(candidates)]);
}

function commitsForPath(repositoryRoot, repositoryPath) {
  const output = git(repositoryRoot, [
    "log",
    "--format=%H",
    "--topo-order",
    "--full-history",
    "HEAD",
    "--",
    repositoryPath,
  ]);
  const commits = output.split("\n").filter(Boolean);
  if (
    commits.length > MAX_PROVENANCE_COMMITS ||
    commits.some((commit) => !/^[0-9a-f]{40}$/u.test(commit)) ||
    new Set(commits).size !== commits.length
  ) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return commits;
}

function isAncestor(repositoryRoot, ancestor, descendant) {
  if (ancestor === descendant) return true;
  try {
    execFileSync(
      "git",
      ["-C", repositoryRoot, "merge-base", "--is-ancestor", ancestor, descendant],
      {
        stdio: "ignore",
      },
    );
    return true;
  } catch (error) {
    if (error !== null && typeof error === "object" && "status" in error && error.status === 1) {
      return false;
    }
    throw new Error("artifact_size_baseline_provenance_invalid", { cause: error });
  }
}

function historicalBaselines(repositoryRoot, repositoryPaths) {
  const byCommit = new Map();
  const invalid = [];
  for (const repositoryPath of repositoryPaths) {
    for (const commit of commitsForPath(repositoryRoot, repositoryPath)) {
      const source = committedFile(repositoryRoot, commit, repositoryPath);
      // A subtree merge can be reported for both sides of the path history even
      // though only one candidate path exists in that merge tree.
      if (source === null) continue;
      let baseline;
      try {
        baseline = validateArtifactSizeBaseline(JSON.parse(source));
      } catch {
        invalid.push(Object.freeze({ commit, repositoryPath }));
        continue;
      }
      const prior = byCommit.get(commit);
      if (prior !== undefined && prior.repositoryPath !== repositoryPath) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
      byCommit.set(commit, Object.freeze({ baseline, commit, repositoryPath }));
    }
  }
  const valid = [...byCommit.values()];
  if (invalid.length > MAX_INVALID_BASELINE_COMMITS) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  for (const candidate of invalid) {
    if (valid.some(({ commit }) => isAncestor(repositoryRoot, commit, candidate.commit))) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
  }
  return valid.sort(
    (left, right) =>
      left.commit.localeCompare(right.commit) ||
      left.repositoryPath.localeCompare(right.repositoryPath),
  );
}

function introductionForDecision(repositoryRoot, historical, decision) {
  const containing = historical.filter(({ baseline }) =>
    baseline.history.some(({ review }) => review.decision === decision),
  );
  let introduction = containing[0];
  if (introduction === undefined) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  for (const candidate of containing.slice(1)) {
    if (isAncestor(repositoryRoot, candidate.commit, introduction.commit)) {
      introduction = candidate;
    }
  }
  if (
    containing.some(
      (candidate) => !isAncestor(repositoryRoot, introduction.commit, candidate.commit),
    )
  ) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return introduction;
}

function validateDecisionSource(repositoryRoot, prefix, entry) {
  const marker = `Decision ID: ${entry.review.sourceDecision}`;
  const matches = repositoryPathCandidates(prefix, entry.review.sourceDecisionPath).filter(
    (repositoryPath) => {
      const source = committedFile(repositoryRoot, entry.review.sourceCommit, repositoryPath);
      return source !== null && source.includes(marker);
    },
  );
  if (matches.length !== 1) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
}

function retainsHistoryThrough(candidate, expected, entryIndex) {
  return (
    candidate.history.length > entryIndex &&
    expected.history
      .slice(0, entryIndex + 1)
      .every((entry, index) => sameEntry(entry, candidate.history[index]))
  );
}

export function validateArtifactSizeBaselineProvenance(value, repositoryRoot) {
  const baseline = validateArtifactSizeBaseline(value);
  const repository = provenanceRepository(repositoryRoot);
  const baselinePaths = repositoryPathCandidates(
    repository.prefix,
    artifactSizeBaselineRepositoryPath,
  );
  const active = baselinePaths.flatMap((repositoryPath) => {
    const committed = committedBaseline(repository.topLevel, "HEAD", repositoryPath);
    return committed === null ? [] : [Object.freeze({ baseline: committed, repositoryPath })];
  });
  if (active.length !== 1 || !sameEntry(active[0].baseline, baseline)) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const historical = historicalBaselines(repository.topLevel, baselinePaths);
  for (const prior of historical) {
    if (
      prior.baseline.history.length > baseline.history.length ||
      prior.baseline.history.some((entry, index) => !sameEntry(entry, baseline.history[index]))
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
  }
  for (const [offset, entry] of baseline.history.slice(1).entries()) {
    const entryIndex = offset + 1;
    const introduced = introductionForDecision(
      repository.topLevel,
      historical,
      entry.review.decision,
    );
    for (const prior of historical) {
      if (
        !retainsHistoryThrough(prior.baseline, baseline, entryIndex) &&
        !isAncestor(repository.topLevel, prior.commit, introduced.commit)
      ) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
    }
    if (
      introduced.commit === entry.review.sourceCommit ||
      !isAncestor(repository.topLevel, entry.review.sourceCommit, introduced.commit)
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    validateDecisionSource(repository.topLevel, repository.prefix, entry);
  }
  return baseline;
}

export function evaluateArtifactBudgets(assets, baselineValue) {
  if (baselineValue === null || baselineValue === undefined) {
    throw new Error("artifact_size_baseline_missing");
  }
  const baseline = validateArtifactSizeBaseline(baselineValue);
  const reviewedEntry = baseline.history.at(-1);
  if (reviewedEntry === undefined) throw new Error("artifact_size_baseline_invalid");
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
    const reviewed = ASYNC_ARTIFACTS.has(role) ? reviewedEntry.roles[role] : undefined;
    const increase =
      reviewed === undefined || !Number.isSafeInteger(bytes) || bytes <= reviewed.brotliBytes
        ? 0
        : ((bytes - reviewed.brotliBytes) / reviewed.brotliBytes) * 100;
    const baselineDetails =
      reviewed === undefined
        ? ""
        : ` baseline=${String(reviewed.brotliBytes)} unreviewed_increase=${increase.toFixed(2)}% threshold=15%`;
    lines.push(
      `artifact_budget role=${role} bytes=${bytes === undefined ? "missing" : String(bytes)} ceiling=${String(ceiling ?? "none")}${baselineDetails}`,
    );
    if (!Number.isSafeInteger(bytes) || bytes < 0) {
      if (asset !== undefined) issues.push(`artifact_budget:bytes:${role}`);
    } else if (ceiling !== null && bytes > ceiling) {
      issues.push(`artifact_budget:${role}:+${String(bytes - ceiling)}`);
    } else if (
      reviewed !== undefined &&
      bytes * 10_000 >
        reviewed.brotliBytes * (10_000 + baseline.maximumUnreviewedIncreaseBasisPoints)
    ) {
      issues.push(
        `artifact_budget:${role}:unreviewed_regression:+${String(bytes - reviewed.brotliBytes)}`,
      );
    }
    if (asset !== undefined && reviewed !== undefined && asset.file !== reviewed.artifact) {
      issues.push(`artifact_budget:baseline_artifact:${role}`);
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
  if (
    candidate.asyncArtifact.sha256 !== runtimeArtifact.asyncSha256 ||
    candidate.asyncArtifact.brotliBytes !== runtimeArtifact.asyncBrotliBytes
  ) {
    throw new Error("browser_budget_candidate_async_artifact_mismatch");
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
  const artifactSizeBaseline = validateArtifactSizeBaselineProvenance(
    await boundedBenchmarkJson(artifactSizeBaselinePath, false),
    resolve(browserRoot, ".."),
  );
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
      sha256: createHash("sha256").update(content).digest("hex"),
    });
  }
  const artifactBudgets = evaluateArtifactBudgets(measured, artifactSizeBaseline);
  process.stdout.write(`${artifactBudgets.lines.join("\n")}\n`);
  if (artifactBudgets.issues.length > 0) {
    throw new Error(`artifact_budget_failed:${artifactBudgets.issues.join(",")}`);
  }
  const runtimeAsset = measured.find(({ role }) => role === "core-esm");
  if (runtimeAsset === undefined) throw new Error("artifact_budget:missing:core-esm");
  const asyncRuntimeAsset = measured.find(({ role }) => role === "async-esm");
  if (asyncRuntimeAsset === undefined) throw new Error("artifact_budget:missing:async-esm");
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
    {
      brotliBytes,
      sha256: runtimeSha256,
      asyncBrotliBytes: asyncRuntimeAsset.brotliBytes,
      asyncSha256: asyncRuntimeAsset.sha256,
    },
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
