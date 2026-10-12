import { createHash, createHmac, randomBytes } from "node:crypto";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

import { buildRuntimeAssets } from "./build.mjs";
import {
  loadCompatibilityMatrix,
  readCompatibilityIdentity,
  validateCompatibilityEvidence,
} from "./check-compatibility.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_MATRIX = resolve(browserRoot, "compatibility/matrix.json");
const DEFAULT_RESULTS = resolve(browserRoot, "compatibility/results");
const HOST = "http://127.0.0.1:4173";
const ADAPTER_KEYS = Object.freeze([
  "attestation",
  "browserProduct",
  "browserVersion",
  "cases",
  "identitySource",
  "operatingSystem",
  "provider",
  "result",
  "testRun",
]);
const RECEIPT_KEYS = Object.freeze([
  "authentication",
  "fixtureManifestSha256",
  "id",
  "nonce",
  "result",
  "runtimeSha256",
]);
const TEST_RUN_KEYS = Object.freeze([
  "authentication",
  "fixtureManifestSha256",
  "nonce",
  "runtimeSha256",
]);
const IDENTITY_SOURCES = Object.freeze({
  chrome: Object.freeze(["cdp_browser_version", "webdriver_capabilities"]),
  edge: Object.freeze(["cdp_browser_version", "webdriver_capabilities"]),
  firefox: Object.freeze(["webdriver_capabilities"]),
  safari: Object.freeze(["safari_webdriver_capabilities"]),
});

class CompatibilityRunnerError extends Error {
  constructor(code) {
    super(code);
    this.name = "CompatibilityRunnerError";
    this.code = code;
  }
}

function runnerError(code) {
  throw new CompatibilityRunnerError(code);
}

function record(value, code) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) runnerError(code);
  return value;
}

function exactKeys(value, expected, code) {
  const actual = Object.keys(value).sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    runnerError(code);
  }
}

function versionMatches(target, actual) {
  if (target.channel === "stable") return true;
  const expected = target.version.split(".");
  const found = actual.split(".");
  return expected.every((part, index) => found[index] === part);
}

function challengeFor(target, matrix, identity) {
  const nonce = randomBytes(32).toString("base64url");
  const secret = randomBytes(32);
  const body = JSON.stringify({
    fixtureManifestSha256: identity.fixtureManifestSha256,
    nonce,
    requiredCases: matrix.requiredCases,
    runtimeSha256: identity.runtimeSha256,
    target: target.id,
  });
  const authentication = createHmac("sha256", secret).update(body).digest("base64url");
  return Object.freeze({
    authentication,
    fixtureManifestSha256: identity.fixtureManifestSha256,
    nonce,
    runtimeSha256: identity.runtimeSha256,
  });
}

function receiptMatches(receipt, expectedId, challenge) {
  const value = record(receipt, "case_receipt_invalid");
  exactKeys(value, RECEIPT_KEYS, "case_receipt_invalid");
  if (
    value.id !== expectedId ||
    (value.result !== "pass" && value.result !== "fail") ||
    value.nonce !== challenge.nonce ||
    value.authentication !== challenge.authentication ||
    value.runtimeSha256 !== challenge.runtimeSha256 ||
    value.fixtureManifestSha256 !== challenge.fixtureManifestSha256
  ) {
    runnerError("case_receipt_invalid");
  }
  return value.result;
}

function validateAdapterResult(raw, target, matrix, challenge) {
  const result = record(raw, "adapter_result_invalid");
  exactKeys(result, ADAPTER_KEYS, "adapter_result_invalid");
  const testRun = record(result.testRun, "test_run_invalid");
  exactKeys(testRun, TEST_RUN_KEYS, "test_run_invalid");
  if (
    testRun.nonce !== challenge.nonce ||
    testRun.authentication !== challenge.authentication ||
    testRun.runtimeSha256 !== challenge.runtimeSha256 ||
    testRun.fixtureManifestSha256 !== challenge.fixtureManifestSha256
  ) {
    runnerError("test_run_invalid");
  }
  if (
    result.browserProduct !== target.browserProduct ||
    typeof result.browserVersion !== "string" ||
    !versionMatches(target, result.browserVersion) ||
    !IDENTITY_SOURCES[target.browserProduct].includes(result.identitySource) ||
    (result.result !== "pass" && result.result !== "fail") ||
    !Array.isArray(result.cases) ||
    result.cases.length !== matrix.requiredCases.length
  ) {
    runnerError("adapter_identity_invalid");
  }
  const byId = new Map();
  for (const receipt of result.cases) {
    const value = record(receipt, "case_receipt_invalid");
    if (typeof value.id !== "string" || byId.has(value.id)) runnerError("case_receipt_invalid");
    byId.set(value.id, value);
  }
  let failed = false;
  for (const id of matrix.requiredCases) {
    const receipt = byId.get(id);
    if (receipt === undefined) runnerError("case_receipt_missing");
    if (receiptMatches(receipt, id, challenge) === "fail") failed = true;
  }
  if ((failed && result.result !== "fail") || (!failed && result.result !== "pass")) {
    runnerError("adapter_result_inconsistent");
  }
  return validateCompatibilityEvidence({
    schemaVersion: 1,
    browserProduct: result.browserProduct,
    browserVersion: result.browserVersion,
    operatingSystem: result.operatingSystem,
    provider: result.provider,
    runtimeSha256: challenge.runtimeSha256,
    fixtureManifestSha256: challenge.fixtureManifestSha256,
    executedAt: new Date().toISOString(),
    result: result.result,
    attestation: result.attestation,
  });
}

async function fetchChecked(url) {
  let response;
  try {
    response = await fetch(url, { cache: "no-store", signal: AbortSignal.timeout(5_000) });
  } catch {
    runnerError("qualification_host_unavailable");
  }
  if (!response.ok || response.headers.get("x-suprnova-conformance-host") !== "1") {
    runnerError("qualification_host_invalid");
  }
  return response;
}

async function verifyHost(baseUrl, identity) {
  const health = await fetchChecked(new URL("/health", baseUrl));
  if ((await health.text()) !== "ok") runnerError("qualification_host_invalid");
  const runtime = await fetchChecked(new URL("/assets/suprnova-live.esm.js", baseUrl));
  const bytes = Buffer.from(await runtime.arrayBuffer());
  if (createHash("sha256").update(bytes).digest("hex") !== identity.runtimeSha256) {
    runnerError("qualification_artifact_mismatch");
  }
  const scenario = await fetchChecked(new URL("/scenario/instance", baseUrl));
  if (!(await scenario.text()).includes("data-suprnova-live-island")) {
    runnerError("qualification_catalog_invalid");
  }
}

async function waitForHost(baseUrl, identity, child) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (child.exitCode !== null) runnerError("qualification_host_failed");
    try {
      await verifyHost(baseUrl, identity);
      return;
    } catch (error) {
      if (
        error instanceof CompatibilityRunnerError &&
        !["qualification_host_unavailable", "qualification_host_invalid"].includes(error.code)
      ) {
        throw error;
      }
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  runnerError("qualification_host_timeout");
}

async function stopHost(child) {
  if (child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolveExit) => child.once("exit", resolveExit)),
    new Promise((resolveDelay) => setTimeout(resolveDelay, 2_000)),
  ]);
  if (child.exitCode === null) child.kill("SIGKILL");
}

function checkedBaseUrl(value) {
  let url;
  try {
    url = new URL(value);
  } catch {
    runnerError("base_url_invalid");
  }
  if (
    (url.protocol !== "http:" && url.protocol !== "https:") ||
    url.username !== "" ||
    url.password !== "" ||
    url.hash !== "" ||
    url.search !== ""
  ) {
    runnerError("base_url_invalid");
  }
  return url;
}

async function adapterFrom(path) {
  let module;
  try {
    module = await import(pathToFileURL(resolve(path)).href);
  } catch {
    runnerError("adapter_unavailable");
  }
  if (typeof module.runCompatibility !== "function") runnerError("adapter_invalid");
  return module.runCompatibility;
}

async function writeEvidence(resultsPath, target, evidence) {
  await mkdir(resultsPath, { recursive: true });
  const destination = resolve(resultsPath, `${target.id}.json`);
  const temporary = resolve(
    resultsPath,
    `.${target.id}.${randomBytes(8).toString("hex")}.temporary`,
  );
  await writeFile(temporary, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
  await rename(temporary, destination);
  return destination;
}

function argumentsFrom(argv) {
  const options = { matrixPath: DEFAULT_MATRIX, resultsPath: DEFAULT_RESULTS };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!["--adapter", "--base-url", "--matrix", "--results", "--target"].includes(argument)) {
      runnerError("usage");
    }
    const value = argv[index + 1];
    if (value === undefined) runnerError("usage");
    index += 1;
    if (argument === "--adapter") options.adapterPath = value;
    else if (argument === "--base-url") options.baseUrl = value;
    else if (argument === "--matrix") options.matrixPath = value;
    else if (argument === "--results") options.resultsPath = value;
    else options.targetId = value;
  }
  if (options.adapterPath === undefined || options.targetId === undefined) runnerError("usage");
  return options;
}

export async function runCompatibility(options) {
  const matrixPath = resolve(options.matrixPath ?? DEFAULT_MATRIX);
  const resultsPath = resolve(options.resultsPath ?? DEFAULT_RESULTS);
  const matrix = await loadCompatibilityMatrix(matrixPath);
  const target = matrix.targets.find((candidate) => candidate.id === options.targetId);
  if (target === undefined) runnerError("target_unknown");
  await buildRuntimeAssets();
  const identity = await readCompatibilityIdentity(matrixPath, matrix);
  if (identity === null) runnerError("current_identity_missing");
  const challenge = challengeFor(target, matrix, identity);
  const adapter = await adapterFrom(options.adapterPath);
  const baseUrl = checkedBaseUrl(options.baseUrl ?? HOST);
  let child = null;
  try {
    if (options.baseUrl === undefined) {
      child = spawn(process.execPath, ["test-host/server.mjs"], {
        cwd: browserRoot,
        stdio: "ignore",
      });
      await waitForHost(baseUrl, identity, child);
    } else await verifyHost(baseUrl, identity);
    let raw;
    try {
      raw = await adapter(
        Object.freeze({
          baseUrl: baseUrl.href,
          challenge,
          requiredCases: matrix.requiredCases,
          target,
        }),
      );
    } catch {
      runnerError("adapter_execution_failed");
    }
    const evidence = validateAdapterResult(raw, target, matrix, challenge);
    return Object.freeze({
      destination: await writeEvidence(resultsPath, target, evidence),
      evidence,
      target: target.id,
    });
  } finally {
    if (child !== null) await stopHost(child);
  }
}

async function main() {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    const result = await runCompatibility(options);
    process.stdout.write(`compatibility evidence written: ${result.target}\n`);
  } catch (error) {
    const code = error instanceof CompatibilityRunnerError ? error.code : "internal";
    process.stderr.write(`compatibility runner failed: ${code}\n`);
    process.exitCode = code === "usage" ? 64 : 1;
  }
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) await main();
