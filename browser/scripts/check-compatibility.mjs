import { createHash } from "node:crypto";
import { lstat, readFile, readdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_MATRIX = resolve(browserRoot, "compatibility/matrix.json");
const MAX_MATRIX_BYTES = 131_072;
const MAX_SCHEMA_BYTES = 131_072;
const MAX_EVIDENCE_BYTES = 65_536;
const MAX_RESULT_FILES = 64;
const SHA256 = /^[a-f0-9]{64}$/u;
const VERSION = /^[0-9]+(?:\.[0-9]+){0,3}$/u;
const CASE_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/u;
const TARGET_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/u;
const PRODUCTS = Object.freeze(["chrome", "edge", "firefox", "safari"]);
const EVIDENCE_KEYS = Object.freeze([
  "attestation",
  "browserProduct",
  "browserVersion",
  "executedAt",
  "fixtureManifestSha256",
  "operatingSystem",
  "provider",
  "result",
  "runtimeSha256",
  "schemaVersion",
]);
const MATRIX_KEYS = Object.freeze([
  "evidenceSchema",
  "fixtureManifest",
  "maxEvidenceAgeDays",
  "requiredCases",
  "runtimeArtifact",
  "schemaVersion",
  "targets",
]);
const TARGET_KEYS = Object.freeze(["browserProduct", "channel", "id", "version"]);
const RESERVED_PROVIDERS = new Set([
  "chromium",
  "local-playwright",
  "playwright",
  "self",
  "self-declared",
  "simulated",
  "webkit",
]);
const EXPECTED_TARGETS = Object.freeze([
  Object.freeze({
    id: "chrome-minimum-111",
    browserProduct: "chrome",
    channel: "minimum",
    version: "111",
  }),
  Object.freeze({
    id: "chrome-current-stable",
    browserProduct: "chrome",
    channel: "stable",
    version: "current",
  }),
  Object.freeze({
    id: "edge-minimum-111",
    browserProduct: "edge",
    channel: "minimum",
    version: "111",
  }),
  Object.freeze({
    id: "edge-current-stable",
    browserProduct: "edge",
    channel: "stable",
    version: "current",
  }),
  Object.freeze({
    id: "firefox-minimum-128",
    browserProduct: "firefox",
    channel: "minimum",
    version: "128",
  }),
  Object.freeze({
    id: "firefox-current-stable",
    browserProduct: "firefox",
    channel: "stable",
    version: "current",
  }),
  Object.freeze({
    id: "safari-minimum-16-4",
    browserProduct: "safari",
    channel: "minimum",
    version: "16.4",
  }),
  Object.freeze({
    id: "safari-current-stable",
    browserProduct: "safari",
    channel: "stable",
    version: "current",
  }),
]);

class CompatibilityInputError extends Error {
  constructor(code) {
    super(code);
    this.name = "CompatibilityInputError";
    this.code = code;
  }
}

function inputError(code) {
  throw new CompatibilityInputError(code);
}

function record(value, code) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) inputError(code);
  return value;
}

function exactKeys(value, expected, code) {
  const actual = Object.keys(value).sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    inputError(code);
  }
}

function containsForbiddenControl(value) {
  return Array.from(value).some((character) => {
    const point = character.codePointAt(0);
    return (
      point !== undefined &&
      (point <= 0x08 ||
        point === 0x0b ||
        point === 0x0c ||
        (point >= 0x0e && point <= 0x1f) ||
        point === 0x7f)
    );
  });
}

function boundedText(value, minimum, maximum, code) {
  if (
    typeof value !== "string" ||
    value.length < minimum ||
    value.length > maximum ||
    value.trim() !== value ||
    containsForbiddenControl(value)
  ) {
    inputError(code);
  }
  return value;
}

async function boundedFile(path, maximum, code) {
  let metadata;
  try {
    metadata = await lstat(path);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return null;
    inputError(code);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > maximum) inputError(code);
  return readFile(path);
}

function parseJson(bytes, code) {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    inputError(code);
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function validateTarget(value, index) {
  const target = record(value, "matrix_target_invalid");
  exactKeys(target, TARGET_KEYS, "matrix_target_invalid");
  const expected = EXPECTED_TARGETS[index];
  if (
    expected === undefined ||
    target.id !== expected.id ||
    target.browserProduct !== expected.browserProduct ||
    target.channel !== expected.channel ||
    target.version !== expected.version ||
    typeof target.id !== "string" ||
    !TARGET_ID.test(target.id)
  ) {
    inputError("matrix_target_invalid");
  }
  return expected;
}

function validateMatrix(value) {
  const matrix = record(value, "matrix_invalid");
  exactKeys(matrix, MATRIX_KEYS, "matrix_invalid");
  if (
    matrix.schemaVersion !== 1 ||
    typeof matrix.evidenceSchema !== "string" ||
    typeof matrix.runtimeArtifact !== "string" ||
    typeof matrix.fixtureManifest !== "string" ||
    !Number.isSafeInteger(matrix.maxEvidenceAgeDays) ||
    matrix.maxEvidenceAgeDays < 1 ||
    matrix.maxEvidenceAgeDays > 365 ||
    !Array.isArray(matrix.requiredCases) ||
    matrix.requiredCases.length === 0 ||
    matrix.requiredCases.length > 128 ||
    !Array.isArray(matrix.targets) ||
    matrix.targets.length !== EXPECTED_TARGETS.length
  ) {
    inputError("matrix_invalid");
  }
  const requiredCases = [];
  const seen = new Set();
  for (const candidate of matrix.requiredCases) {
    if (typeof candidate !== "string" || !CASE_ID.test(candidate) || seen.has(candidate)) {
      inputError("matrix_case_invalid");
    }
    seen.add(candidate);
    requiredCases.push(candidate);
  }
  const targets = matrix.targets.map(validateTarget);
  return Object.freeze({
    evidenceSchema: matrix.evidenceSchema,
    fixtureManifest: matrix.fixtureManifest,
    maxEvidenceAgeDays: matrix.maxEvidenceAgeDays,
    requiredCases: Object.freeze(requiredCases),
    runtimeArtifact: matrix.runtimeArtifact,
    schemaVersion: 1,
    targets: Object.freeze(targets),
  });
}

function validateEvidenceSchema(value) {
  const schema = record(value, "evidence_schema_invalid");
  const properties = record(schema.properties, "evidence_schema_invalid");
  if (
    schema.$schema !== "https://json-schema.org/draft/2020-12/schema" ||
    schema.type !== "object" ||
    schema.additionalProperties !== false ||
    !Array.isArray(schema.required) ||
    schema.required.length !== EVIDENCE_KEYS.length ||
    [...schema.required].sort().some((key, index) => key !== EVIDENCE_KEYS[index]) ||
    Object.keys(properties)
      .sort()
      .some((key, index) => key !== EVIDENCE_KEYS[index]) ||
    Object.keys(properties).length !== EVIDENCE_KEYS.length
  ) {
    inputError("evidence_schema_invalid");
  }
  const product = record(properties.browserProduct, "evidence_schema_invalid");
  if (!Array.isArray(product.enum) || product.enum.join(",") !== PRODUCTS.join(",")) {
    inputError("evidence_schema_invalid");
  }
}

function parseVersion(value, code) {
  if (typeof value !== "string" || !VERSION.test(value)) inputError(code);
  const parts = value.split(".").map((part) => Number.parseInt(part, 10));
  if (parts.some((part) => !Number.isSafeInteger(part))) inputError(code);
  return parts;
}

function matchesFloor(actual, floor) {
  const actualParts = parseVersion(actual, "evidence_browser_version_invalid");
  const floorParts = parseVersion(floor, "matrix_target_invalid");
  return floorParts.every((part, index) => actualParts[index] === part);
}

function validateEvidence(value) {
  const evidence = record(value, "evidence_invalid");
  exactKeys(evidence, EVIDENCE_KEYS, "evidence_invalid");
  if (
    evidence.schemaVersion !== 1 ||
    !PRODUCTS.includes(evidence.browserProduct) ||
    (evidence.result !== "pass" && evidence.result !== "fail") ||
    typeof evidence.runtimeSha256 !== "string" ||
    !SHA256.test(evidence.runtimeSha256) ||
    typeof evidence.fixtureManifestSha256 !== "string" ||
    !SHA256.test(evidence.fixtureManifestSha256)
  ) {
    inputError("evidence_invalid");
  }
  parseVersion(evidence.browserVersion, "evidence_browser_version_invalid");
  boundedText(evidence.operatingSystem, 1, 128, "evidence_os_invalid");
  const provider = boundedText(evidence.provider, 1, 128, "evidence_provider_invalid");
  const normalizedProvider = provider.trim().toLowerCase();
  if (
    RESERVED_PROVIDERS.has(normalizedProvider) ||
    normalizedProvider.includes("simulated") ||
    normalizedProvider.includes("user-agent")
  ) {
    inputError("evidence_provider_invalid");
  }
  boundedText(evidence.attestation, 16, 4096, "evidence_attestation_invalid");
  boundedText(evidence.executedAt, 20, 40, "evidence_timestamp_invalid");
  const executedAt = Date.parse(evidence.executedAt);
  if (!Number.isFinite(executedAt) || new Date(executedAt).toISOString() !== evidence.executedAt) {
    inputError("evidence_timestamp_invalid");
  }
  return Object.freeze({ ...evidence, executedAtMilliseconds: executedAt });
}

export function validateCompatibilityEvidence(value) {
  const validated = validateEvidence(value);
  return Object.freeze({
    schemaVersion: validated.schemaVersion,
    browserProduct: validated.browserProduct,
    browserVersion: validated.browserVersion,
    operatingSystem: validated.operatingSystem,
    provider: validated.provider,
    runtimeSha256: validated.runtimeSha256,
    fixtureManifestSha256: validated.fixtureManifestSha256,
    executedAt: validated.executedAt,
    result: validated.result,
    attestation: validated.attestation,
  });
}

function targetEvidenceDisposition(target, evidence, context) {
  if (evidence.browserProduct !== target.browserProduct) return "product_mismatch";
  if (target.channel === "minimum" && !matchesFloor(evidence.browserVersion, target.version)) {
    return "version_mismatch";
  }
  if (evidence.runtimeSha256 !== context.runtimeSha256) return "runtime_stale";
  if (evidence.fixtureManifestSha256 !== context.fixtureManifestSha256) {
    return "fixtures_stale";
  }
  if (evidence.executedAtMilliseconds > context.now + 300_000) return "timestamp_future";
  if (context.now - evidence.executedAtMilliseconds > context.maximumAgeMilliseconds) {
    return "evidence_stale";
  }
  if (evidence.result === "fail") return "conformance_failed";
  return "qualified";
}

function classification(code) {
  return ["runtime_stale", "fixtures_stale", "evidence_stale"].includes(code)
    ? "unqualified"
    : "failed";
}

export async function loadCompatibilityMatrix(path = DEFAULT_MATRIX) {
  const resolved = resolve(path);
  const bytes = await boundedFile(resolved, MAX_MATRIX_BYTES, "matrix_unreadable");
  if (bytes === null) inputError("matrix_missing");
  return validateMatrix(parseJson(bytes, "matrix_invalid"));
}

export async function readCompatibilityIdentity(matrixPath, matrix, overrides = {}) {
  const base = dirname(matrixPath);
  const runtimePath = overrides.runtimePath ?? resolve(base, matrix.runtimeArtifact);
  const fixtureManifestPath =
    overrides.fixtureManifestPath ?? resolve(base, matrix.fixtureManifest);
  const runtime = await boundedFile(runtimePath, 4_194_304, "runtime_artifact_invalid");
  if (runtime === null) return null;
  const fixture = await boundedFile(fixtureManifestPath, 4096, "fixture_manifest_invalid");
  if (fixture === null) return null;
  const fixtureManifestSha256 = fixture.toString("utf8").trim();
  if (!SHA256.test(fixtureManifestSha256)) inputError("fixture_manifest_invalid");
  return Object.freeze({ fixtureManifestSha256, runtimeSha256: sha256(runtime) });
}

export async function checkCompatibility(options = {}) {
  const matrixPath = resolve(options.matrixPath ?? DEFAULT_MATRIX);
  const matrix = await loadCompatibilityMatrix(matrixPath);
  const schemaPath = resolve(dirname(matrixPath), matrix.evidenceSchema);
  const schema = await boundedFile(schemaPath, MAX_SCHEMA_BYTES, "evidence_schema_unreadable");
  if (schema === null) inputError("evidence_schema_missing");
  validateEvidenceSchema(parseJson(schema, "evidence_schema_invalid"));
  const resultsPath = resolve(options.resultsPath ?? resolve(dirname(matrixPath), "results"));
  const identity = await readCompatibilityIdentity(matrixPath, matrix, options);
  if (identity === null) {
    return Object.freeze({
      status: "unqualified",
      qualified: 0,
      required: matrix.targets.length,
      details: Object.freeze([{ target: "matrix", code: "current_identity_missing" }]),
    });
  }
  let entries;
  try {
    entries = await readdir(resultsPath, { withFileTypes: true });
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") entries = [];
    else inputError("results_unreadable");
  }
  const jsonEntries = entries.filter((entry) => entry.name.endsWith(".json"));
  if (jsonEntries.length > MAX_RESULT_FILES) inputError("result_file_limit");
  const expectedFiles = new Set(matrix.targets.map((target) => `${target.id}.json`));
  const details = [];
  let qualified = 0;
  let status = "qualified";
  for (const entry of jsonEntries) {
    if (!entry.isFile() || !expectedFiles.has(entry.name)) {
      details.push({ target: "matrix", code: "unexpected_evidence_file" });
      status = "failed";
    }
  }
  const nowDate = options.now === undefined ? new Date() : new Date(options.now);
  const now = nowDate.getTime();
  if (!Number.isFinite(now)) inputError("clock_invalid");
  const context = Object.freeze({
    ...identity,
    maximumAgeMilliseconds: matrix.maxEvidenceAgeDays * 86_400_000,
    now,
  });
  for (const target of matrix.targets) {
    const bytes = await boundedFile(
      resolve(resultsPath, `${target.id}.json`),
      MAX_EVIDENCE_BYTES,
      "evidence_unreadable",
    );
    if (bytes === null) {
      details.push({ target: target.id, code: "evidence_missing" });
      if (status !== "failed") status = "unqualified";
      continue;
    }
    try {
      const evidence = validateEvidence(parseJson(bytes, "evidence_invalid"));
      const disposition = targetEvidenceDisposition(target, evidence, context);
      if (disposition === "qualified") {
        qualified += 1;
        continue;
      }
      details.push({ target: target.id, code: disposition });
      const evidenceStatus = classification(disposition);
      if (evidenceStatus === "failed" || status === "qualified") status = evidenceStatus;
    } catch (error) {
      const code = error instanceof CompatibilityInputError ? error.code : "evidence_invalid";
      details.push({ target: target.id, code });
      status = "failed";
    }
  }
  if (qualified === matrix.targets.length && details.length === 0) status = "qualified";
  return Object.freeze({
    status,
    qualified,
    required: matrix.targets.length,
    details: Object.freeze(details.map((detail) => Object.freeze(detail))),
  });
}

function argumentsFrom(argv) {
  const options = { allowUnqualified: false, json: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--allow-unqualified") options.allowUnqualified = true;
    else if (argument === "--json") options.json = true;
    else if (
      ["--matrix", "--results", "--runtime", "--fixture-manifest", "--now"].includes(argument)
    ) {
      const value = argv[index + 1];
      if (value === undefined) inputError("usage");
      index += 1;
      if (argument === "--matrix") options.matrixPath = value;
      else if (argument === "--results") options.resultsPath = value;
      else if (argument === "--runtime") options.runtimePath = value;
      else if (argument === "--fixture-manifest") options.fixtureManifestPath = value;
      else options.now = value;
    } else inputError("usage");
  }
  return options;
}

async function main() {
  let options;
  try {
    options = argumentsFrom(process.argv.slice(2));
    const result = await checkCompatibility(options);
    if (options.json) process.stdout.write(`${JSON.stringify(result)}\n`);
    else {
      process.stdout.write(
        `compatibility qualification: ${result.status} (${String(result.qualified)}/${String(result.required)})\n`,
      );
    }
    if (
      result.status === "qualified" ||
      (result.status === "unqualified" && options.allowUnqualified)
    ) {
      return;
    }
    process.exitCode = result.status === "failed" ? 1 : 2;
  } catch (error) {
    const code = error instanceof CompatibilityInputError ? error.code : "internal";
    process.stderr.write(`compatibility qualification failed: ${code}\n`);
    process.exitCode = code === "usage" ? 64 : 1;
  }
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) await main();
