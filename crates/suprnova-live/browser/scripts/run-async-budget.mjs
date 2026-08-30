import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { readdir, readFile } from "node:fs/promises";
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
const CHILD_TIMEOUT_MILLISECONDS = 240_000;
const HEAP_SAMPLES_PER_STATE = 5;
const RETENTION_MUTATIONS = new Set([
  "none",
  "large_island_buffer",
  "predecessor_transport",
  "stale_current_payload",
  "stale_queued_payload",
]);

export class AsyncBudgetRunnerError extends Error {
  constructor(code, options) {
    super(code, options);
    this.code = code;
  }
}

function fail(code) {
  throw new AsyncBudgetRunnerError(code);
}

function retainedPrimaryError(primary, cleanupErrors) {
  if (cleanupErrors.length === 0) return primary;
  const cause = new AggregateError(
    [primary, ...cleanupErrors],
    "async_budget_cleanup_after_failure",
  );
  return primary instanceof AsyncBudgetRunnerError
    ? new AsyncBudgetRunnerError(primary.code, { cause })
    : new AggregateError([primary, ...cleanupErrors], "async_budget_operation_and_cleanup_failed", {
        cause: primary,
      });
}

async function cleanupResources(steps, primary) {
  const cleanupErrors = [];
  for (const step of steps) {
    if (step.resource === null) continue;
    try {
      await step.close(step.resource);
    } catch (error) {
      cleanupErrors.push(error);
    }
  }
  if (primary !== null) throw retainedPrimaryError(primary, cleanupErrors);
  if (cleanupErrors.length > 0) {
    throw new AsyncBudgetRunnerError("async_budget_cleanup_failed", {
      cause: new AggregateError(cleanupErrors, "async_budget_cleanup_failed"),
    });
  }
}

export async function withAsyncBudgetBrowserResources(dependencies, operation) {
  let server = null;
  let browser = null;
  let context = null;
  let result;
  let primary = null;
  try {
    server = dependencies.createServer();
    const baseUrl = await dependencies.listen(server);
    browser = await dependencies.launch();
    context = await dependencies.newContext(browser);
    result = await operation({ baseUrl, browser, context });
  } catch (error) {
    primary = error;
  } finally {
    await cleanupResources(
      [
        { close: dependencies.closeContext, resource: context },
        { close: dependencies.closeBrowser, resource: browser },
        { close: dependencies.closeServer, resource: server },
      ],
      primary,
    );
  }
  return result;
}

export async function withAsyncBudgetPageResources(context, dependencies, operation) {
  let page = null;
  let session = null;
  let result;
  let primary = null;
  try {
    page = await dependencies.newPage(context);
    session = await dependencies.newSession(context, page);
    result = await operation({ page, session });
  } catch (error) {
    primary = error;
  } finally {
    await cleanupResources(
      [
        { close: dependencies.detachSession, resource: session },
        { close: dependencies.closePage, resource: page },
      ],
      primary,
    );
  }
  return result;
}

export function argumentsFrom(argv) {
  const options = {
    artifact: null,
    baseline: DEFAULT_BASELINE,
    child: false,
    output: DEFAULT_OUTPUT,
    profile: "reduced",
    recordExploratory: false,
    retentionMutation: "none",
    serverOutput: DEFAULT_SERVER_OUTPUT,
    verifyRetentionMutations: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--child") options.child = true;
    else if (argument === "--record-exploratory") options.recordExploratory = true;
    else if (argument === "--verify-retention-mutations") {
      options.verifyRetentionMutations = true;
    } else if (
      [
        "--artifact",
        "--baseline",
        "--output",
        "--profile",
        "--retention-mutation",
        "--server-output",
      ].includes(argument)
    ) {
      const value = argv[index + 1];
      if (value === undefined) fail("usage");
      index += 1;
      if (argument === "--artifact") options.artifact = resolve(value);
      else if (argument === "--baseline") options.baseline = resolve(value);
      else if (argument === "--output") options.output = resolve(value);
      else if (argument === "--retention-mutation") options.retentionMutation = value;
      else if (argument === "--server-output") options.serverOutput = resolve(value);
      else options.profile = value;
    } else fail("usage");
  }
  if (options.profile !== "reduced" && options.profile !== "qualified") fail("profile_invalid");
  if (!RETENTION_MUTATIONS.has(options.retentionMutation)) fail("retention_mutation_invalid");
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

async function cpuGovernor() {
  try {
    const entries = await readdir("/sys/devices/system/cpu", { withFileTypes: true });
    const governors = new Set();
    for (const entry of entries) {
      if (!entry.isDirectory() || !/^cpu[0-9]+$/u.test(entry.name)) continue;
      try {
        governors.add(
          (
            await readFile(`/sys/devices/system/cpu/${entry.name}/cpufreq/scaling_governor`, "utf8")
          ).trim(),
        );
      } catch {
        // CPUs without cpufreq support do not qualify but do not hide other governors.
      }
    }
    return governors.size === 0 ? "unavailable" : [...governors].sort().join("+");
  } catch {
    return "unavailable";
  }
}

async function heapSamples(session) {
  const samples = [];
  for (let index = 0; index < HEAP_SAMPLES_PER_STATE; index += 1) {
    await session.send("HeapProfiler.collectGarbage");
    const sample = await session.send("Runtime.getHeapUsage");
    const values = [sample.usedSize, sample.embedderHeapUsedSize, sample.backingStorageSize];
    if (values.some((value) => !Number.isSafeInteger(value) || value < 0)) {
      fail("async_heap_measurement_invalid");
    }
    samples.push({
      backingStorageSize: sample.backingStorageSize,
      embedderHeapUsedSize: sample.embedderHeapUsedSize,
      usedSize: sample.usedSize,
    });
  }
  return samples;
}

async function retentionSession(page) {
  return page.evaluate(() => {
    const value = Reflect.get(globalThis, "__suprnovaBudgetAsyncRetention");
    if (typeof value !== "object" || value === null) {
      throw new Error("async_retention_session_missing");
    }
    return true;
  });
}

async function measureRetention(page, session, phase, baseline) {
  await retentionSession(page);
  let postWorkload;
  let liveResources;
  try {
    postWorkload = await heapSamples(session);
    liveResources = await page.evaluate(() =>
      Reflect.get(globalThis, "__suprnovaBudgetAsyncRetention").resources(),
    );
  } finally {
    await page.evaluate(() => Reflect.get(globalThis, "__suprnovaBudgetAsyncRetention").cleanup());
  }
  const cleanupResources = await page.evaluate(() =>
    Reflect.get(globalThis, "__suprnovaBudgetAsyncRetention").resources(),
  );
  await page.evaluate(() => {
    Reflect.deleteProperty(globalThis, "__suprnovaBudgetAsyncRetention");
    Reflect.deleteProperty(globalThis, "__suprnovaBudgetAsyncRetentionGate");
  });
  const cleanup = await heapSamples(session);
  return Object.freeze({
    baseline,
    cleanup,
    cleanupResources,
    liveResources,
    phase,
    postWorkload,
  });
}

async function measurePage(
  context,
  baseUrl,
  artifactSha256,
  workloadSource,
  options = { checkpoint: null, retentionMutation: "none" },
) {
  return withAsyncBudgetPageResources(
    context,
    {
      closePage: (page) => page.close(),
      detachSession: (session) => session.detach(),
      newPage: (owner) => owner.newPage(),
      newSession: (owner, page) => owner.newCDPSession(page),
    },
    async ({ page, session }) => {
      await session.send("Emulation.setCPUThrottlingRate", { rate: 4 });
      const version = await session.send("Browser.getVersion");
      if (typeof version.protocolVersion !== "string" || typeof version.product !== "string") {
        fail("async_cdp_version_unavailable");
      }
      await page.goto(`${baseUrl}/health`);
      await page.setContent(
        `<!doctype html><html lang="en"><body>${Array.from(
          { length: 100 },
          (_, index) => `<section data-async-benchmark-index="${String(index)}"></section>`,
        ).join("")}</body></html>`,
        { waitUntil: "domcontentloaded" },
      );
      await page.addScriptTag({ content: workloadSource });
      const measurementPromise = page.evaluate(
        async ({ artifactUrl, checkpoint, expectedArtifactSha256, retentionMutation }) => {
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
          if (checkpoint !== null) {
            let release;
            const released = new Promise((resolvePromise) => {
              release = resolvePromise;
            });
            const gate = {
              ready: false,
              release() {
                release();
              },
              wait() {
                gate.ready = true;
                return released;
              },
            };
            Reflect.set(globalThis, "__suprnovaBudgetAsyncRetentionGate", gate);
          }
          return benchmark.measureAsyncWorkloads({
            ...input,
            ...(checkpoint === null ? {} : { retentionCheckpoint: checkpoint }),
            retentionMutation,
          });
        },
        {
          artifactUrl: `${baseUrl}/suprnova-live.async.esm.js`,
          checkpoint: options.checkpoint,
          expectedArtifactSha256: artifactSha256,
          retentionMutation: options.retentionMutation,
        },
      );
      let baseline = null;
      if (options.checkpoint !== null) {
        const ready = page.waitForFunction(
          () => Reflect.get(globalThis, "__suprnovaBudgetAsyncRetentionGate")?.ready === true,
        );
        await Promise.race([
          ready,
          measurementPromise.then(() => fail("async_retention_baseline_gate_skipped")),
        ]);
        baseline = await heapSamples(session);
        await page.evaluate(() =>
          Reflect.get(globalThis, "__suprnovaBudgetAsyncRetentionGate").release(),
        );
      }
      const measurement = await measurementPromise;
      const retention =
        options.checkpoint === null
          ? null
          : await measureRetention(page, session, options.checkpoint, baseline);
      return Object.freeze({
        measurement,
        protocol: Object.freeze({
          method: "Runtime.getHeapUsage",
          product: version.product,
          protocolVersion: version.protocolVersion,
        }),
        retention,
      });
    },
  );
}

async function measureMutationProofs(context, baseUrl, artifactSha256, workloadSource, protocol) {
  const largeIslandBuffer = await measurePage(context, baseUrl, artifactSha256, workloadSource, {
    checkpoint: "e100",
    retentionMutation: "large_island_buffer",
  });
  const predecessorTransport = await measurePage(context, baseUrl, artifactSha256, workloadSource, {
    checkpoint: "r100",
    retentionMutation: "predecessor_transport",
  });
  const staleCurrentPayload = await measurePage(context, baseUrl, artifactSha256, workloadSource, {
    checkpoint: "r100",
    retentionMutation: "stale_current_payload",
  });
  const staleQueuedPayload = await measurePage(context, baseUrl, artifactSha256, workloadSource, {
    checkpoint: "r100",
    retentionMutation: "stale_queued_payload",
  });
  const mutationRuns = [
    largeIslandBuffer,
    predecessorTransport,
    staleCurrentPayload,
    staleQueuedPayload,
  ];
  if (mutationRuns.some((run) => JSON.stringify(run.protocol) !== JSON.stringify(protocol))) {
    fail("async_cdp_version_changed");
  }
  const predecessorMeasurement = predecessorTransport.measurement.R100;
  if (predecessorMeasurement === null) fail("async_retention_mutation_probe_invalid");
  if (
    predecessorTransport.retention === null ||
    staleCurrentPayload.retention === null ||
    staleQueuedPayload.retention === null ||
    largeIslandBuffer.retention === null
  ) {
    fail("async_retention_mutation_probe_invalid");
  }
  return Object.freeze({
    largeIslandBuffer: Object.freeze({
      artifactSha256,
      documentTransports: largeIslandBuffer.measurement.E100.physicalConnectionCount,
      phase: "E100",
      productPath: "AsyncSubscription.pending",
      retention: largeIslandBuffer.retention,
      subscriptionId: "subscription-000",
    }),
    predecessorTransport: Object.freeze({
      artifactSha256,
      physicalTransportsAfterCurrent: predecessorMeasurement.physicalTransportsAfterCurrent,
      productPath: "AsyncDocumentOwner.transport",
      reconnectHandshakes: predecessorMeasurement.documentReconnectHandshakes,
      retention: predecessorTransport.retention,
    }),
    staleCurrentPayload: Object.freeze({
      artifactSha256,
      phase: "R100",
      productPath: "AsyncSubscription.activeRefresh",
      retention: staleCurrentPayload.retention,
      subscriptionId: "subscription-000",
    }),
    staleQueuedPayload: Object.freeze({
      artifactSha256,
      phase: "R100",
      productPath: "AsyncSubscription.pending",
      retention: staleQueuedPayload.retention,
      subscriptionId: "subscription-000",
    }),
  });
}

async function childMeasurement(artifactPath, retentionMutation, verifyRetentionMutations) {
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
  return withAsyncBudgetBrowserResources(
    {
      closeBrowser: (browser) => browser.close(),
      closeContext: (context) => context.close(),
      closeServer,
      createServer: () =>
        createServer((request, response) => {
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
        }),
      launch: () => chromium.launch({ headless: true }),
      listen,
      newContext: (browser) => browser.newContext({ viewport: { height: 720, width: 1_280 } }),
    },
    async ({ baseUrl, browser, context }) => {
      const baseEnvironment = await browserEnvironment(
        browser,
        process.env.SUPRNOVA_LIVE_B1_DEDICATED === "1",
      );
      const governor = await cpuGovernor();
      const qualified = baseEnvironment.qualificationRequirementsMet && governor === "performance";
      const environment = Object.freeze({
        ...baseEnvironment,
        classification: qualified ? "qualified" : "unqualified",
        governor,
        providerProfile: "rust-owner-browser-measured-source-v1",
        qualificationRequirementsMet: qualified,
      });
      await measurePage(context, baseUrl, artifactSha256, workload.source);
      const e100 = await measurePage(context, baseUrl, artifactSha256, workload.source, {
        checkpoint: "e100",
        retentionMutation,
      });
      const r100 = await measurePage(context, baseUrl, artifactSha256, workload.source, {
        checkpoint: "r100",
        retentionMutation,
      });
      if (e100.measurement.R100 !== null || r100.measurement.R100 === null) {
        fail("async_retention_checkpoint_invalid");
      }
      if (JSON.stringify(e100.protocol) !== JSON.stringify(r100.protocol)) {
        fail("async_cdp_version_changed");
      }
      const mutationProofs = verifyRetentionMutations
        ? await measureMutationProofs(
            context,
            baseUrl,
            artifactSha256,
            workload.source,
            e100.protocol,
          )
        : null;
      return Object.freeze({
        artifactSha256,
        environment,
        measurement: Object.freeze({
          E100: e100.measurement.E100,
          R100: r100.measurement.R100,
        }),
        mutationProofs,
        processId: process.pid,
        protocol: e100.protocol,
        retention: Object.freeze({ E100: e100.retention, R100: r100.retention }),
      });
    },
  );
}

function runChild(artifactPath, retentionMutation, verifyRetentionMutations) {
  const childArguments = [
    fileURLToPath(import.meta.url),
    "--child",
    "--artifact",
    artifactPath,
    "--retention-mutation",
    retentionMutation,
  ];
  if (verifyRetentionMutations) childArguments.push("--verify-retention-mutations");
  const execution = spawnSync(process.execPath, childArguments, {
    cwd: browserRoot,
    encoding: "utf8",
    env: process.env,
    maxBuffer: MAX_JSON_BYTES,
    timeout: CHILD_TIMEOUT_MILLISECONDS,
  });
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

function sameProtocol(runs) {
  const encoded = JSON.stringify(runs[0]?.protocol);
  if (encoded === undefined || runs.some((run) => JSON.stringify(run.protocol) !== encoded)) {
    fail("async_cdp_version_changed");
  }
  return runs[0].protocol;
}

function derivedRetention(raw, helpers) {
  if (
    raw === null ||
    raw.liveResources?.subscriptions === undefined ||
    raw.cleanupResources?.subscriptions === undefined
  ) {
    fail("async_retention_measurement_missing");
  }
  const cleanupSubscriptions = raw.cleanupResources.subscriptions;
  if (
    raw.cleanupResources.activeTransportOwners !== 0 ||
    raw.cleanupResources.currentPayloadOwners !== 0 ||
    raw.cleanupResources.predecessorContinuityOwners !== 0 ||
    raw.cleanupResources.predecessorTransportOwners !== 0 ||
    raw.cleanupResources.queuedPayloadOwners !== 0 ||
    cleanupSubscriptions.some(
      (entry) =>
        entry.authorizationBytes !== 0 ||
        entry.currentPayloadBytes !== 0 ||
        entry.currentPayloadOwners !== 0 ||
        entry.queuedPayloadBytes !== 0 ||
        entry.queuedPayloadOwners !== 0,
    )
  ) {
    fail("async_retention_cleanup_incomplete");
  }
  return Object.freeze({
    ...helpers.derivePostWorkloadRetention({
      baseline: raw.baseline,
      cleanup: raw.cleanup,
      postWorkload: raw.postWorkload,
      subscriptions: raw.liveResources.subscriptions,
    }),
    cleanupResources: raw.cleanupResources,
    liveResources: raw.liveResources,
  });
}

function selectedRetention(runs, phase, helpers) {
  const candidates = runs.map((run) => derivedRetention(run.retention?.[phase], helpers));
  return candidates.reduce((selected, candidate) =>
    Math.max(...candidate.subscriptions.map((entry) => entry.retainedBytes)) >
    Math.max(...selected.subscriptions.map((entry) => entry.retainedBytes))
      ? candidate
      : selected,
  );
}

function mutationProofEvidence(runs, artifactSha256, helpers) {
  const proofRuns = runs.filter((run) => run.mutationProofs !== null);
  if (proofRuns.length !== 1) fail("async_retention_mutation_proof_count");
  const proof = proofRuns[0]?.mutationProofs;
  if (proof === undefined) fail("async_retention_mutation_proof_missing");
  for (const entry of Object.values(proof)) {
    if (entry?.artifactSha256 !== artifactSha256) fail("async_retention_mutation_artifact");
  }
  return Object.freeze({
    largeIslandBuffer: Object.freeze({
      ...proof.largeIslandBuffer,
      retention: derivedRetention(proof.largeIslandBuffer.retention, helpers),
    }),
    predecessorTransport: Object.freeze({
      ...proof.predecessorTransport,
      retention: derivedRetention(proof.predecessorTransport.retention, helpers),
    }),
    staleCurrentPayload: Object.freeze({
      ...proof.staleCurrentPayload,
      retention: derivedRetention(proof.staleCurrentPayload.retention, helpers),
    }),
    staleQueuedPayload: Object.freeze({
      ...proof.staleQueuedPayload,
      retention: derivedRetention(proof.staleQueuedPayload.retention, helpers),
    }),
  });
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
  const protocol = sameProtocol(runs);
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
  const e100Retention = selectedRetention(runs, "E100", helpers);
  const r100Retention = selectedRetention(runs, "R100", helpers);
  const subscriptions = dispatchRun.measurement.E100.subscriptions.map((subscription) => ({
    ...subscription,
    retention: e100Retention.subscriptions.find((entry) => entry.id === subscription.id),
  }));
  const recovery = recoveryRun.measurement.R100.recovery.map((entry) => ({
    ...entry,
    retention: r100Retention.subscriptions.find((candidate) => candidate.id === entry.id),
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
          activeTransportOwners: e100Retention.liveResources.activeTransportOwners,
          currentPayloadOwners: e100Retention.liveResources.currentPayloadOwners,
          fairnessMaximumLead: maximum(runs, (run) => run.measurement.E100.fairnessMaximumLead),
          handshakes: maximum(runs, (run) => run.measurement.E100.handshakeCount),
          maxQueuedBytes: maximum(runs, (run) => run.measurement.E100.queuedBytePeak),
          maxQueuedEvents: maximum(runs, (run) => run.measurement.E100.queuedEventPeak),
          physicalTransports: maximum(runs, (run) => run.measurement.E100.physicalConnectionCount),
          queuedPayloadOwners: e100Retention.liveResources.queuedPayloadOwners,
          starvedSubscriptions:
            100 - Math.min(...runs.map((run) => run.measurement.E100.currentSubscriptionCount)),
        },
        retention: e100Retention,
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
      retainedHeap: {
        api: "Chromium CDP Runtime.getHeapUsage",
        baselineState:
          "same page, DOM, benchmark harness, loaded production artifact, and provider scaffolding before original controller activation",
        cleanupState:
          "all original controllers disposed, all transport and payload owners released, retention session removed",
        derivation:
          "total=max(post_workload)-min(baseline); shared=max(0,total-actual_owner_bytes); per_island=ceil(shared/100)+actual_owner_bytes",
        exclusions: ["DOM", "benchmark_harness", "released_current_payload"],
        garbageCollection: "HeapProfiler.collectGarbage",
        harnessTreatment:
          "same-page DOM and harness exist in baseline and post-workload; unmatched runtime/native transport growth is conservatively included in shared structural bytes",
        phaseSamples: HEAP_SAMPLES_PER_STATE,
        postWorkloadState:
          "all 100 original workloaded controllers remain live and current before any disposal",
        product: protocol.product,
        protocolVersion: protocol.protocolVersion,
        unavailable: "fail_closed",
      },
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
    mutationProofs: mutationProofEvidence(runs, artifactEvidence.sha256, helpers),
    r100: {
      bounds: {
        maxConcurrentHandshakesPerOrigin: 8,
        maxRetainedBytesAfterCurrent: 12 * 1_024,
        reconnectHandshakes: 1,
      },
      measurements: {
        document: {
          currentPayloadOwners: r100Retention.liveResources.currentPayloadOwners,
          generationAfter: recoveryRun.measurement.R100.generationAfter,
          generationBefore: recoveryRun.measurement.R100.generationBefore,
          maximumConcurrentReauthorizations: maximum(
            runs,
            (run) => run.measurement.R100.maximumConcurrentReauthorizations,
          ),
          physicalTransportsAfterCurrent:
            recoveryRun.measurement.R100.physicalTransportsAfterCurrent,
          predecessorContinuityOwners: r100Retention.liveResources.predecessorContinuityOwners,
          predecessorTransportOwners: r100Retention.liveResources.predecessorTransportOwners,
          queuedPayloadOwners: r100Retention.liveResources.queuedPayloadOwners,
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
        retention: r100Retention,
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
        e100RetainedMaximumBytes: Math.max(
          ...derivedRetention(run.retention.E100, helpers).subscriptions.map(
            (entry) => entry.retainedBytes,
          ),
        ),
        evidenceSha256: sha256(
          Buffer.from(
            JSON.stringify({
              measurement: run.measurement,
              mutationProofs: run.mutationProofs,
              retention: run.retention,
            }),
          ),
        ),
        processId: run.processId,
        r100RetainedMaximumBytes: Math.max(
          ...derivedRetention(run.retention.R100, helpers).subscriptions.map(
            (entry) => entry.retainedBytes,
          ),
        ),
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
  const runs = Array.from({ length: runsRequired }, (_, index) =>
    runChild(artifactPath, options.retentionMutation, index === 0),
  );
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
    ...result.e100.measurements.subscriptions.map(
      (subscription) => subscription.retention.retainedBytes,
    ),
  );
  const retainedAfterRecovery = Math.max(
    ...result.r100.measurements.recovery.map((entry) => entry.retention.retainedBytes),
  );
  process.stdout.write(
    `E100/1K+R100 async budget classification=${evaluation.classification} dispatch_p50=${String(result.e100.measurements.dispatch.p50Milliseconds)}ms dispatch_p95=${String(result.e100.measurements.dispatch.p95Milliseconds)}ms recovery_p50=${String(result.r100.measurements.timeToCurrent.p50Milliseconds)}ms recovery_p95=${String(result.r100.measurements.timeToCurrent.p95Milliseconds)}ms retained_max=${String(retained)}B retained_after_recovery_max=${String(retainedAfterRecovery)}B transport=${String(result.e100.measurements.document.physicalTransports)} reconnect=${String(result.r100.measurements.document.reconnectHandshakes)} scheduler_max=${String(result.multiDocument.maximumConcurrentHandshakes)} artifact_brotli=${String(result.artifact.brotliBytes)}B output=${options.output}\n`,
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
      process.stdout.write(
        JSON.stringify(
          await childMeasurement(
            options.artifact,
            options.retentionMutation,
            options.verifyRetentionMutations,
          ),
        ),
      );
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
