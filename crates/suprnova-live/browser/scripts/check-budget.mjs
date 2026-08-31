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
const MAX_INVALID_BASELINE_OBJECTS = 8;
const MAX_PROVENANCE_COMMITS = 256;
const MAX_PROVENANCE_PARENTS = 8;
const MAX_PROVENANCE_PATHS = 128;
const MAX_REPOSITORY_PATH_BYTES = 1_024;
const MAX_COMMITTED_FILE_BYTES = 1024 * 1024;
const MAX_COMMITTED_OBJECT_QUERIES = 768;
const MAX_COMMITTED_OBJECT_RESPONSE_BYTES = 16 * 1024 * 1024;
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
  const output = git(repositoryRoot, ["rev-parse", "--show-toplevel", "--show-prefix"]);
  const lines = output.split("\n");
  if (lines.length < 1 || lines.length > 2 || lines[0] === "") {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const topLevel = lines[0];
  const rawPrefix = lines[1] ?? "";
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

function graphNodes(output, allowBoundary) {
  const lines = output.split("\n").filter(Boolean);
  if (lines.length < 1 || lines.length > MAX_PROVENANCE_COMMITS) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const nodes = new Map();
  for (const line of lines) {
    const boundary = line.startsWith("-");
    const fields = (boundary ? line.slice(1) : line).split(" ");
    if (
      (!allowBoundary && boundary) ||
      fields.length < 1 ||
      fields.length > MAX_PROVENANCE_PARENTS + 1 ||
      fields.some((commit) => !/^[0-9a-f]{40}$/u.test(commit)) ||
      nodes.has(fields[0])
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    nodes.set(
      fields[0],
      Object.freeze({ boundary, parents: Object.freeze(boundary ? [] : fields.slice(1)) }),
    );
  }
  return nodes;
}

function provenanceGraph(repositoryRoot, repositoryPaths, sourceAnchor) {
  const pathOutput = git(repositoryRoot, [
    "rev-list",
    "--parents",
    "--full-history",
    "--simplify-merges",
    `--max-count=${String(MAX_PROVENANCE_COMMITS + 1)}`,
    "--topo-order",
    "HEAD",
    "--",
    ...repositoryPaths,
  ]);
  const nodes = graphNodes(pathOutput, false);
  if (sourceAnchor !== null) {
    const sourceOutput = git(repositoryRoot, [
      "rev-list",
      "--parents",
      "--ancestry-path",
      "--boundary",
      `--max-count=${String(MAX_PROVENANCE_COMMITS + 1)}`,
      "--topo-order",
      "HEAD",
      `^${sourceAnchor}`,
    ]);
    for (const [commit, sourceNode] of graphNodes(sourceOutput, true)) {
      const existing = nodes.get(commit);
      if (existing === undefined || !sourceNode.boundary) nodes.set(commit, sourceNode);
    }
  }
  if (nodes.size > MAX_PROVENANCE_COMMITS) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  for (const { parents } of nodes.values()) {
    if (parents.some((parent) => !nodes.has(parent))) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
  }
  const visiting = new Set();
  const ancestors = new Map();
  const collectAncestors = (commit) => {
    const memoized = ancestors.get(commit);
    if (memoized !== undefined) return memoized;
    if (visiting.has(commit)) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    visiting.add(commit);
    const result = new Set([commit]);
    const node = nodes.get(commit);
    if (node === undefined) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    for (const parent of node.parents) {
      for (const ancestor of collectAncestors(parent)) result.add(ancestor);
    }
    visiting.delete(commit);
    const frozen = Object.freeze(result);
    ancestors.set(commit, frozen);
    return frozen;
  };
  for (const commit of nodes.keys()) collectAncestors(commit);
  return Object.freeze({
    commits: Object.freeze([...nodes.keys()]),
    isAncestor(ancestor, descendant) {
      const reachable = ancestors.get(descendant);
      if (reachable === undefined || !nodes.has(ancestor)) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
      return reachable.has(ancestor);
    },
    parents(commit) {
      const node = nodes.get(commit);
      if (node === undefined) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
      return node.parents;
    },
    contains(commit) {
      return nodes.has(commit);
    },
  });
}

function committedObjects(repositoryRoot, requestedQueries) {
  const queries = [...new Set(requestedQueries)].sort();
  if (queries.length < 1 || queries.length > MAX_COMMITTED_OBJECT_QUERIES) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  let output;
  try {
    output = execFileSync("git", ["-C", repositoryRoot, "cat-file", "--batch"], {
      encoding: null,
      input: `${queries.join("\n")}\n`,
      maxBuffer: MAX_COMMITTED_OBJECT_RESPONSE_BYTES,
      stdio: ["pipe", "pipe", "ignore"],
    });
  } catch (error) {
    throw new Error("artifact_size_baseline_provenance_invalid", { cause: error });
  }
  if (!Buffer.isBuffer(output)) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const objects = new Map();
  let offset = 0;
  for (const query of queries) {
    const headerEnd = output.indexOf(10, offset);
    if (headerEnd < 0) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    const header = output.subarray(offset, headerEnd).toString("utf8");
    offset = headerEnd + 1;
    if (header === `${query} missing`) {
      objects.set(query, null);
      continue;
    }
    const match = /^([0-9a-f]{40}) blob ([0-9]+)$/u.exec(header);
    const size = match === null ? Number.NaN : Number(match[2]);
    if (
      match === null ||
      !Number.isSafeInteger(size) ||
      size < 0 ||
      size > MAX_COMMITTED_FILE_BYTES ||
      offset + size >= output.length ||
      output[offset + size] !== 10
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    objects.set(
      query,
      Object.freeze({
        objectId: match[1],
        source: output.subarray(offset, offset + size).toString(),
      }),
    );
    offset += size + 1;
  }
  if (offset !== output.length) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return objects;
}

function committedObject(objects, commit, repositoryPath) {
  const query = `${commit}:${repositoryPath}`;
  if (!objects.has(query)) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return objects.get(query);
}

function baselineState(objects, commit, repositoryPaths) {
  const present = repositoryPaths.flatMap((repositoryPath) => {
    const object = committedObject(objects, commit, repositoryPath);
    return object === null ? [] : [Object.freeze({ object, repositoryPath })];
  });
  if (present.length > 1) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const file = present[0];
  if (file === undefined) return Object.freeze({ kind: "missing" });
  try {
    return Object.freeze({
      baseline: validateArtifactSizeBaseline(JSON.parse(file.object.source)),
      kind: "valid",
      objectId: file.object.objectId,
      repositoryPath: file.repositoryPath,
    });
  } catch {
    return Object.freeze({
      kind: "invalid",
      objectId: file.object.objectId,
      repositoryPath: file.repositoryPath,
    });
  }
}

function historicalBaselines(graph, objects, repositoryPaths) {
  const currentPath = repositoryPaths.length === 2 ? repositoryPaths[0] : null;
  const legacyPath = repositoryPaths.at(-1);
  const authority = new Map();
  const historical = [];
  const invalidObjects = new Set();
  const resolveAuthority = (commit) => {
    if (authority.has(commit)) return authority.get(commit);
    const parentStates = graph.parents(commit).map(resolveAuthority);
    const authorityParents = parentStates.filter((parent) => parent !== null);
    const state = baselineState(objects, commit, repositoryPaths);
    if (authorityParents.length === 0) {
      if (state.kind === "invalid") invalidObjects.add(state.objectId);
      const introduced =
        state.kind === "valid"
          ? Object.freeze({
              baseline: state.baseline,
              commit,
              relocated: false,
              repositoryPath: state.repositoryPath,
            })
          : null;
      authority.set(commit, introduced);
      if (introduced !== null) historical.push(introduced);
      return introduced;
    }
    if (state.kind !== "valid") {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    const parentPaths = new Set(authorityParents.map(({ repositoryPath }) => repositoryPath));
    const parentRelocations = new Set(authorityParents.map(({ relocated }) => relocated));
    if (parentPaths.size !== 1 || parentRelocations.size !== 1) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    const parentPath = authorityParents[0].repositoryPath;
    const wasRelocated = authorityParents[0].relocated;
    const pathChanged = state.repositoryPath !== parentPath;
    if (
      pathChanged &&
      (wasRelocated ||
        legacyPath === undefined ||
        currentPath === null ||
        parentPath !== legacyPath ||
        state.repositoryPath !== currentPath)
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    for (const parent of authorityParents) {
      if (
        parent.baseline.history.length > state.baseline.history.length ||
        parent.baseline.history.some(
          (entry, index) => !sameEntry(entry, state.baseline.history[index]),
        )
      ) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
    }
    const accepted = Object.freeze({
      baseline: state.baseline,
      commit,
      relocated: wasRelocated || pathChanged,
      repositoryPath: state.repositoryPath,
    });
    authority.set(commit, accepted);
    historical.push(accepted);
    return accepted;
  };
  for (const commit of graph.commits) resolveAuthority(commit);
  if (invalidObjects.size > MAX_INVALID_BASELINE_OBJECTS) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return Object.freeze(historical);
}

function introductionForDecision(graph, historical, decision) {
  const containing = historical.filter(({ baseline }) =>
    baseline.history.some(({ review }) => review.decision === decision),
  );
  const introductions = containing.filter((candidate) =>
    containing.every((descendant) => graph.isAncestor(candidate.commit, descendant.commit)),
  );
  if (introductions.length !== 1) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  return introductions[0];
}

function exactDecisionMarker(source, decision) {
  const marker = `Decision ID: ${decision}.`;
  return source.split(/\r?\n/u).some((line) => {
    const match = /^- [0-9]{4}-[0-9]{2}-[0-9]{2} -- (.*)$/u.exec(line);
    return match !== null && match[1] === marker;
  });
}

function validateDecisionSource(objects, prefix, entry) {
  const matches = repositoryPathCandidates(prefix, entry.review.sourceDecisionPath).filter(
    (repositoryPath) => {
      const object = committedObject(objects, entry.review.sourceCommit, repositoryPath);
      return object !== null && exactDecisionMarker(object.source, entry.review.sourceDecision);
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
  const decisionPaths = baseline.history
    .slice(1)
    .flatMap((entry) =>
      repositoryPathCandidates(repository.prefix, entry.review.sourceDecisionPath),
    );
  const repositoryPaths = [...new Set([...baselinePaths, ...decisionPaths])].sort();
  if (repositoryPaths.length < 1 || repositoryPaths.length > MAX_PROVENANCE_PATHS) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const graph = provenanceGraph(
    repository.topLevel,
    repositoryPaths,
    baseline.history[1]?.review.sourceCommit ?? null,
  );
  for (const entry of baseline.history.slice(1)) {
    if (!graph.contains(entry.review.sourceCommit)) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
  }
  const queries = [
    ...baselinePaths.map((repositoryPath) => `HEAD:${repositoryPath}`),
    ...graph.commits.flatMap((commit) =>
      baselinePaths.map((repositoryPath) => `${commit}:${repositoryPath}`),
    ),
    ...baseline.history
      .slice(1)
      .flatMap((entry) =>
        repositoryPathCandidates(repository.prefix, entry.review.sourceDecisionPath).map(
          (repositoryPath) => `${entry.review.sourceCommit}:${repositoryPath}`,
        ),
      ),
  ];
  const objects = committedObjects(repository.topLevel, queries);
  const active = baselineState(objects, "HEAD", baselinePaths);
  if (active.kind !== "valid" || !sameEntry(active.baseline, baseline)) {
    throw new Error("artifact_size_baseline_provenance_invalid");
  }
  const historical = historicalBaselines(graph, objects, baselinePaths);
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
    const introduced = introductionForDecision(graph, historical, entry.review.decision);
    for (const prior of historical) {
      if (
        !retainsHistoryThrough(prior.baseline, baseline, entryIndex) &&
        !graph.isAncestor(prior.commit, introduced.commit)
      ) {
        throw new Error("artifact_size_baseline_provenance_invalid");
      }
    }
    if (
      introduced.commit === entry.review.sourceCommit ||
      !graph.isAncestor(entry.review.sourceCommit, introduced.commit)
    ) {
      throw new Error("artifact_size_baseline_provenance_invalid");
    }
    validateDecisionSource(objects, repository.prefix, entry);
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
