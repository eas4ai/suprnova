import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { chromium } from "@playwright/test";
import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";
import { atomicWriteEvidence, browserEnvironment, bundledModule } from "./run-upload-budget.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(browserRoot, "..");
const DEFAULT_BASELINE = resolve(browserRoot, "benchmarks/baselines/async-budget-v1.json");
const DEFAULT_OUTPUT = resolve(browserRoot, "benchmarks/local/async-budget-v1.json");
const DEFAULT_SERVER_OUTPUT = resolve(repositoryRoot, "benchmarks/local/async-server-v1.json");
const DRIVER_MARKER = "SUPRNOVA_ASYNC_BUDGET_DRIVER_V1";
const MAX_JSON_BYTES = 4 * 1_024 * 1_024;
const CHILD_TIMEOUT_MILLISECONDS = 120_000;

export class AsyncBudgetRunnerError extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}

function fail(code) {
  throw new AsyncBudgetRunnerError(code);
}

export function argumentsFrom(argv) {
  const options = {
    artifact: null,
    baseline: DEFAULT_BASELINE,
    child: false,
    output: DEFAULT_OUTPUT,
    profile: "reduced",
    recordExploratory: false,
    serverOutput: DEFAULT_SERVER_OUTPUT,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--child") options.child = true;
    else if (argument === "--record-exploratory") options.recordExploratory = true;
    else if (
      ["--artifact", "--baseline", "--output", "--profile", "--server-output"].includes(argument)
    ) {
      const value = argv[index + 1];
      if (value === undefined) fail("usage");
      index += 1;
      if (argument === "--artifact") options.artifact = resolve(value);
      else if (argument === "--baseline") options.baseline = resolve(value);
      else if (argument === "--output") options.output = resolve(value);
      else if (argument === "--server-output") options.serverOutput = resolve(value);
      else options.profile = value;
    } else fail("usage");
  }
  if (options.profile !== "reduced" && options.profile !== "qualified") fail("profile_invalid");
  if (options.child && options.artifact === null) fail("usage");
  if (!options.child && options.artifact !== null) fail("usage");
  if (options.output === options.baseline) fail("baseline_overwrite_forbidden");
  if (options.recordExploratory && options.profile !== "reduced") {
    fail("exploratory_record_requires_reduced_profile");
  }
  return Object.freeze(options);
}

async function boundedJson(path, missingAllowed = false) {
  let bytes;
  try {
    bytes = await readFile(path);
  } catch (error) {
    if (missingAllowed && error instanceof Error && "code" in error && error.code === "ENOENT") {
      return null;
    }
    fail("evidence_unreadable");
  }
  if (bytes.byteLength > MAX_JSON_BYTES) fail("evidence_unreadable");
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    fail("evidence_invalid");
  }
}

async function importedBundle(entryPoint) {
  const { source } = await bundledModule(entryPoint, "node", "esm");
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function verifyArtifactBinding(artifactBytes, manifestBytes) {
  let manifest;
  try {
    manifest = JSON.parse(Buffer.from(manifestBytes).toString("utf8"));
  } catch {
    fail("artifact_manifest_invalid");
  }
  const assets = Array.isArray(manifest?.assets) ? manifest.assets : [];
  const exact = assets.filter(
    (asset) => asset?.file === "suprnova-live.async.esm.js" && asset?.role === "async-esm",
  );
  if (exact.length !== 1 || exact[0].sha256 !== sha256(artifactBytes)) {
    fail("artifact_manifest_mismatch");
  }
  return Object.freeze({
    manifestSha256: sha256(manifestBytes),
    sha256: exact[0].sha256,
  });
}

async function sourceInputsSha256() {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    entryPoints: [resolve(browserRoot, "src/entry-async-esm.ts")],
    format: "esm",
    legalComments: "none",
    metafile: true,
    minify: false,
    platform: "browser",
    target: ["chrome111"],
    treeShaking: true,
    write: false,
  });
  const inputs = Object.keys(result.metafile.inputs).sort();
  if (
    inputs.length === 0 ||
    inputs.some((input) => input.includes("benchmarks/") || input.includes("tests/"))
  ) {
    fail("production_artifact_input_invalid");
  }
  const digest = createHash("sha256");
  for (const input of inputs) {
    digest.update(input);
    digest.update("\0");
    digest.update(await readFile(resolve(browserRoot, input)));
    digest.update("\0");
  }
  return digest.digest("hex");
}

async function listen(server) {
  await new Promise((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  const address = server.address();
  if (typeof address !== "object" || address === null) fail("artifact_server_address_invalid");
  return `http://127.0.0.1:${String(address.port)}`;
}

async function closeServer(server) {
  await new Promise((resolvePromise, reject) => {
    server.close((error) => (error === undefined ? resolvePromise() : reject(error)));
  });
}

async function measurePage(context, baseUrl, artifactSha256, workloadSource) {
  const page = await context.newPage();
  const session = await context.newCDPSession(page);
  try {
    await session.send("Emulation.setCPUThrottlingRate", { rate: 4 });
    await page.goto(`${baseUrl}/health`);
    await page.setContent(
      `<!doctype html><html lang="en"><body>${Array.from(
        { length: 100 },
        (_, index) => `<section data-async-benchmark-index="${String(index)}"></section>`,
      ).join("")}</body></html>`,
      { waitUntil: "domcontentloaded" },
    );
    await page.addScriptTag({ content: workloadSource });
    return await page.evaluate(
      async ({ artifactUrl, expectedArtifactSha256 }) => {
        const benchmark = globalThis.SuprnovaAsyncBudgetDriver;
        if (benchmark?.ASYNC_BUDGET_DRIVER_MARKER !== "SUPRNOVA_ASYNC_BUDGET_DRIVER_V1") {
          throw new Error("async_budget_driver_missing");
        }
        const input = {
          artifactUrl,
          expectedArtifactSha256,
          eventEnvelopeBytes: 1_024,
          multiDocumentCount: 16,
          presentationEventCount: 1_000,
          refreshInvalidationCount: 100,
          scheduledDurationMs: 10_000,
          subscriptionCount: 100,
        };
        await benchmark.measureAsyncWorkloads({ ...input, prepare: true });
        return benchmark.measureAsyncWorkloads(input);
      },
      {
        artifactUrl: `${baseUrl}/suprnova-live.async.esm.js`,
        expectedArtifactSha256: artifactSha256,
      },
    );
  } finally {
    await session.detach();
    await page.close();
  }
}

async function childMeasurement(artifactPath) {
  const artifact = await readFile(artifactPath);
  const artifactSha256 = sha256(artifact);
  const workload = await bundledModule(
    "benchmarks/async-budget-driver.ts",
    "browser",
    "iife",
    "SuprnovaAsyncBudgetDriver",
  );
  if (!workload.source.includes(DRIVER_MARKER)) fail("async_budget_driver_invalid");
  if (
    workload.inputs.some(
      (input) => input.includes("src/async-updates/") || input.includes("src/features/"),
    )
  ) {
    fail("benchmark_bundle_contains_production_implementation");
  }
  const server = createServer((request, response) => {
    if (request.url === "/suprnova-live.async.esm.js") {
      response.writeHead(200, {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      });
      response.end(artifact);
      return;
    }
    if (request.url === "/health") {
      response.writeHead(200, { "content-type": "text/plain; charset=utf-8" });
      response.end("ok");
      return;
    }
    response.writeHead(404);
    response.end();
  });
  const baseUrl = await listen(server);
  const browser = await chromium.launch({ headless: true });
  try {
    const environment = await browserEnvironment(
      browser,
      process.env.SUPRNOVA_LIVE_B1_DEDICATED === "1",
    );
    const context = await browser.newContext({ viewport: { height: 720, width: 1_280 } });
    try {
      await measurePage(context, baseUrl, artifactSha256, workload.source);
      const measurement = await measurePage(context, baseUrl, artifactSha256, workload.source);
      return Object.freeze({
        artifactSha256,
        environment,
        measurement,
        processId: process.pid,
      });
    } finally {
      await context.close();
    }
  } finally {
    await browser.close();
    await closeServer(server);
  }
}

function runChild(artifactPath) {
  const execution = spawnSync(
    process.execPath,
    [fileURLToPath(import.meta.url), "--child", "--artifact", artifactPath],
    {
      cwd: browserRoot,
      encoding: "utf8",
      env: process.env,
      maxBuffer: MAX_JSON_BYTES,
      timeout: CHILD_TIMEOUT_MILLISECONDS,
    },
  );
  const failure = childExecutionFailure(
    execution,
    "async_budget_watchdog",
    "async_budget_child_failed",
  );
  if (failure !== null) {
    process.stderr.write(`${execution.stderr}`.slice(-8_192));
    fail(failure);
  }
  try {
    return JSON.parse(execution.stdout);
  } catch {
    fail("async_budget_child_invalid");
  }
}

export function childExecutionFailure(execution, watchdogCode, failureCode) {
  if (execution.error?.code === "ETIMEDOUT") return watchdogCode;
  if (execution.error !== undefined || execution.status !== 0) return failureCode;
  return null;
}

function sameEnvironment(runs) {
  const encoded = JSON.stringify(runs[0]?.environment);
  if (encoded === undefined || runs.some((run) => JSON.stringify(run.environment) !== encoded)) {
    fail("browser_environment_changed_between_runs");
  }
  return runs[0].environment;
}

function maximum(runs, select) {
  return Math.max(...runs.map(select));
}

function selectedRun(runs, select) {
  return runs.reduce((selected, run) => (select(run) > select(selected) ? run : selected));
}

export function exactServerEvidence(value, artifactSha256) {
  if (
    value?.schemaVersion !== 1 ||
    value?.suite !== "E100/1K" ||
    value?.artifactSha256 !== artifactSha256 ||
    !Number.isSafeInteger(value?.processId) ||
    typeof value?.evidence !== "object" ||
    value.evidence === null
  ) {
    fail("async_server_evidence_invalid");
  }
  return value.evidence;
}

async function runServerProof(artifactSha256, output) {
  const execution = spawnSync("cargo", ["bench", "--bench", "async_framework_budget"], {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CARGO_INCREMENTAL: "0",
      CARGO_BUILD_JOBS: "2",
      SUPRNOVA_LIVE_ASYNC_ARTIFACT_SHA256: artifactSha256,
      SUPRNOVA_LIVE_ASYNC_SERVER_OUTPUT: output,
    },
    maxBuffer: 1_048_576,
    timeout: CHILD_TIMEOUT_MILLISECONDS,
  });
  const failure = childExecutionFailure(
    execution,
    "async_server_proof_watchdog",
    "async_server_proof_failed",
  );
  if (failure !== null) {
    process.stderr.write(`${execution.stdout}${execution.stderr}`.slice(-8_192));
    fail(failure);
  }
  return exactServerEvidence(await boundedJson(output), artifactSha256);
}

function mergeEvidence(runs, artifactEvidence, serverEvidence, helpers) {
  const environment = sameEnvironment(runs);
  const dispatchRun = selectedRun(
    runs,
    (run) =>
      helpers.summarizeAsyncSamples(run.measurement.E100.dispatchEffectSamplesMs).p95Milliseconds,
  );
  const recoveryRun = selectedRun(
    runs,
    (run) => helpers.summarizeAsyncSamples(run.measurement.R100.recoverySamplesMs).p95Milliseconds,
  );
  const dispatch = helpers.summarizeAsyncSamples(
    dispatchRun.measurement.E100.dispatchEffectSamplesMs,
  );
  const timeToCurrent = helpers.summarizeAsyncSamples(
    recoveryRun.measurement.R100.recoverySamplesMs,
  );
  const subscriptions = dispatchRun.measurement.E100.subscriptions.map((subscription) => {
    const retainedCategories = {
      authorizationBytes: subscription.authorizationBytes,
      identifierBytes: subscription.identifierBytes,
      pendingBytes: subscription.pendingBytes,
      pendingEvents: subscription.pendingEvents,
      pollTimers: subscription.pollTimers,
      refreshSlots: subscription.refreshSlots,
      runtimeRecords: subscription.runtimeRecords,
    };
    return {
      current: subscription.current,
      dispatches: subscription.dispatches,
      finalEpoch: subscription.finalEpoch,
      finalSequence: subscription.finalSequence,
      id: subscription.id,
      maxInFlightRefreshes: subscription.maxInFlightRefreshes,
      maxQueuedRefreshes: subscription.maxQueuedRefreshes,
      presentationEvents: subscription.presentationEvents,
      refreshInvalidations: subscription.refreshInvalidations,
      retainedBytes: helpers.estimateAsyncRetainedBytes(retainedCategories),
      retainedCategories,
    };
  });
  const recovery = recoveryRun.measurement.R100.recovery.map((entry) => ({
    ...entry,
    retainedBytes: helpers.estimateAsyncRetainedBytes(entry.retainedCategories),
  }));
  const pollCounts = new Map();
  for (const due of recoveryRun.measurement.R100.pollDueMilliseconds) {
    pollCounts.set(due, (pollCounts.get(due) ?? 0) + 1);
  }
  const multiRun = selectedRun(
    runs,
    (run) => run.measurement.R100.multiDocument.maximumConcurrentHandshakes,
  );
  return {
    artifact: artifactEvidence,
    e100: {
      bounds: {
        maxDispatchP95Milliseconds: 8,
        maxDocumentBytes: 256 * 1_024,
        maxDocumentEvents: 64,
        maxInFlightRefreshesPerIsland: 1,
        maxQueuedRefreshesPerIsland: 1,
        maxRetainedBytesPerSubscription: 8 * 1_024,
      },
      measurements: {
        dispatch,
        document: {
          fairnessMaximumLead: maximum(runs, (run) => run.measurement.E100.fairnessMaximumLead),
          handshakes: maximum(runs, (run) => run.measurement.E100.handshakeCount),
          maxQueuedBytes: maximum(runs, (run) => run.measurement.E100.queuedBytePeak),
          maxQueuedEvents: maximum(runs, (run) => run.measurement.E100.queuedEventPeak),
          physicalTransports: maximum(runs, (run) => run.measurement.E100.physicalConnectionCount),
          starvedSubscriptions:
            100 - Math.min(...runs.map((run) => run.measurement.E100.currentSubscriptionCount)),
        },
        rustOwner: serverEvidence,
        subscriptions,
      },
      workload: {
        documentTransports: 1,
        durationMs: 10_000,
        payloadBytes: 1_024,
        presentationEvents: 1_000,
        refreshInvalidations: 100,
        subscriptions: 100,
      },
    },
    environment,
    methodology: {
      controlledTimeline: true,
      independentRuns: runs.length,
      measuredSamples: 1_000,
      monotonicClock: "performance.now",
      regressionReference: "median_run_p95_v1",
      warmupIterations: 1,
      watchdogOutsideSamples: true,
    },
    multiDocument: {
      attemptedHandshakes: 16,
      completedHandshakes: multiRun.measurement.R100.multiDocument.completedHandshakes,
      documentCount: 16,
      label: "separate_multi_document_scheduler",
      maximumConcurrentHandshakes:
        multiRun.measurement.R100.multiDocument.maximumConcurrentHandshakes,
      origin: "single_origin_controlled_scheduler",
      startOrder: multiRun.measurement.R100.multiDocument.startOrder,
    },
    r100: {
      bounds: {
        maxConcurrentHandshakesPerOrigin: 8,
        maxRetainedBytesAfterCurrent: 12 * 1_024,
        reconnectHandshakes: 1,
      },
      measurements: {
        document: {
          generationAfter: recoveryRun.measurement.R100.generationAfter,
          generationBefore: recoveryRun.measurement.R100.generationBefore,
          maximumConcurrentReauthorizations: maximum(
            runs,
            (run) => run.measurement.R100.maximumConcurrentReauthorizations,
          ),
          physicalTransportsAfterCurrent:
            recoveryRun.measurement.R100.physicalTransportsAfterCurrent,
          reconnectHandshakes: maximum(
            runs,
            (run) => run.measurement.R100.documentReconnectHandshakes,
          ),
          recoveredSubscriptions: Math.min(
            ...runs.map((run) => run.measurement.R100.recoveredSubscriptionCount),
          ),
          starvedSubscriptions: maximum(
            runs,
            (run) => run.measurement.R100.starvedSubscriptionCount,
          ),
        },
        polling: {
          buckets: [...pollCounts].map(([dueMilliseconds, count]) => ({ count, dueMilliseconds })),
          maximumSameTick: maximum(runs, (run) => run.measurement.R100.pollingMaximumSameTick),
        },
        reconnectJitter: {
          buckets: [
            {
              count: 1,
              delayMilliseconds: recoveryRun.measurement.R100.reconnectDelayMilliseconds,
            },
          ],
          handshakes: recoveryRun.measurement.R100.documentReconnectHandshakes,
        },
        recovery,
        timeToCurrent,
      },
      workload: {
        reconnectHandshakes: 1,
        simultaneousContinuityLosses: 100,
        subscriptions: 100,
      },
    },
    recordedAt: new Date().toISOString(),
    runs: runs.map((run, index) => {
      const dispatchSummary = helpers.summarizeAsyncSamples(
        run.measurement.E100.dispatchEffectSamplesMs,
      );
      const recoverySummary = helpers.summarizeAsyncSamples(run.measurement.R100.recoverySamplesMs);
      return {
        artifactSha256: artifactEvidence.sha256,
        dispatchP95Milliseconds: dispatchSummary.p95Milliseconds,
        evidenceSha256: sha256(Buffer.from(JSON.stringify(run.measurement))),
        processId: run.processId,
        recoveryP95Milliseconds: recoverySummary.p95Milliseconds,
        runIndex: index + 1,
      };
    }),
    schemaVersion: 1,
    suite: "E100/1K+R100",
  };
}

async function parentMain(options) {
  await buildRuntimeAssets();
  const artifactPath = resolve(browserRoot, "dist/suprnova-live.async.esm.js");
  const manifestPath = resolve(browserRoot, "dist/suprnova-live.assets.json");
  const artifact = await readFile(artifactPath);
  const manifest = await readFile(manifestPath);
  if (artifact.includes(DRIVER_MARKER)) fail("benchmark_observer_in_production_artifact");
  const binding = verifyArtifactBinding(artifact, manifest);
  const artifactEvidence = {
    brotliBytes: brotliCompressSync(artifact, {
      params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
    }).byteLength,
    file: "suprnova-live.async.esm.js",
    manifestSha256: binding.manifestSha256,
    role: "async-esm",
    sha256: binding.sha256,
    sourceInputsSha256: await sourceInputsSha256(),
  };
  const serverEvidence = await runServerProof(artifactEvidence.sha256, options.serverOutput);
  const runsRequired = options.profile === "qualified" ? 3 : 1;
  const runs = Array.from({ length: runsRequired }, () => runChild(artifactPath));
  const helpers = await importedBundle("benchmarks/async-budget-workloads.ts");
  const schema = await importedBundle("benchmarks/async-budget-schema.ts");
  const result = schema.validateAsyncBudgetEvidence(
    mergeEvidence(runs, artifactEvidence, serverEvidence, helpers),
  );
  const baselineValue = await boundedJson(options.baseline, true);
  let baseline;
  if (options.recordExploratory) {
    const qualifiedBaseline = baselineValue?.qualifiedBaseline ?? null;
    baseline = schema.validateAsyncBudgetBaseline({
      exploratoryReference: result,
      qualifiedBaseline,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    });
  } else {
    if (baselineValue === null) fail("async_budget_baseline_missing");
    baseline = schema.validateAsyncBudgetBaseline(baselineValue);
  }
  const evaluation = schema.evaluateAsyncBudget(result, baseline, {
    release: options.profile === "qualified",
  });
  await atomicWriteEvidence(
    options.output,
    `${JSON.stringify(result, null, 2)}\n`,
    baselineValue === null ? null : options.baseline,
  );
  if (options.recordExploratory && evaluation.issues.length === 0) {
    await atomicWriteEvidence(options.baseline, `${JSON.stringify(baseline, null, 2)}\n`, null);
  }
  const retained = Math.max(
    ...result.e100.measurements.subscriptions.map((subscription) => subscription.retainedBytes),
  );
  process.stdout.write(
    `E100/1K+R100 async budget classification=${evaluation.classification} dispatch_p50=${String(result.e100.measurements.dispatch.p50Milliseconds)}ms dispatch_p95=${String(result.e100.measurements.dispatch.p95Milliseconds)}ms recovery_p50=${String(result.r100.measurements.timeToCurrent.p50Milliseconds)}ms recovery_p95=${String(result.r100.measurements.timeToCurrent.p95Milliseconds)}ms retained_max=${String(retained)}B transport=${String(result.e100.measurements.document.physicalTransports)} reconnect=${String(result.r100.measurements.document.reconnectHandshakes)} scheduler_max=${String(result.multiDocument.maximumConcurrentHandshakes)} artifact_brotli=${String(result.artifact.brotliBytes)}B output=${options.output}\n`,
  );
  if (evaluation.observations.length > 0) {
    process.stdout.write(`E100/1K+R100 observations: ${evaluation.observations.join(",")}\n`);
  }
  if (evaluation.issues.length > 0) {
    process.stderr.write(`E100/1K+R100 async budget failed: ${evaluation.issues.join(",")}\n`);
    process.exitCode = 1;
  }
}

async function main() {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    if (options.child) {
      process.stdout.write(JSON.stringify(await childMeasurement(options.artifact)));
      return;
    }
    await parentMain(options);
  } catch (error) {
    const code = error instanceof AsyncBudgetRunnerError ? error.code : "internal";
    if (code === "internal" && error instanceof Error) process.stderr.write(`${error.stack}\n`);
    process.stderr.write(`E100/1K+R100 async budget runner failed: ${code}\n`);
    process.exitCode = code === "usage" ? 64 : 1;
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
