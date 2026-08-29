import { createHash, randomBytes } from "node:crypto";
import { lstat, mkdir, open, readFile, realpath, rename, stat, unlink } from "node:fs/promises";
import { arch, cpus, platform, release, totalmem } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { chromium } from "@playwright/test";
import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(browserRoot, "..");
const DEFAULT_BASELINE = resolve(browserRoot, "benchmarks/baselines/upload-budget-v1.json");
const DEFAULT_OUTPUT = resolve(browserRoot, "benchmarks/local/upload-budget-v1.json");
const DEFAULT_SERVER = resolve(repositoryRoot, "benchmarks/local/upload-server-v1.json");
const OBSERVER_MARKER = "suprnova-upload-budget-observer-v1";
const MAX_JSON_BYTES = 1_048_576;

export class UploadBudgetRunnerError extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}

function fail(code) {
  throw new UploadBudgetRunnerError(code);
}

async function optionalMetadata(path, operation) {
  try {
    return await operation(path);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return null;
    throw error;
  }
}

async function rejectBaselineAlias(destination, protectedPath) {
  if (protectedPath === null) return;
  await mkdir(dirname(destination), { recursive: true });
  const destinationLink = await optionalMetadata(destination, lstat);
  if (destinationLink?.isSymbolicLink()) fail("baseline_overwrite_forbidden");
  const protectedReal = await realpath(protectedPath);
  const destinationReal =
    destinationLink === null
      ? join(await realpath(dirname(destination)), basename(destination))
      : await realpath(destination);
  if (destinationReal === protectedReal) fail("baseline_overwrite_forbidden");
  const destinationStat = await optionalMetadata(destination, stat);
  const protectedStat = await stat(protectedPath);
  if (
    destinationStat !== null &&
    destinationStat.dev === protectedStat.dev &&
    destinationStat.ino === protectedStat.ino
  ) {
    fail("baseline_overwrite_forbidden");
  }
}

export async function atomicWriteEvidence(
  destination,
  contents,
  protectedPath,
  { failStage = "none" } = {},
) {
  await rejectBaselineAlias(destination, protectedPath);
  const parent = dirname(destination);
  await mkdir(parent, { recursive: true });
  const temporary = join(
    parent,
    `.${basename(destination)}.tmp-${String(process.pid)}-${randomBytes(8).toString("hex")}`,
  );
  let handle;
  try {
    handle = await open(temporary, "wx", 0o600);
    if (failStage === "after_partial_write") {
      const bytes = Buffer.from(contents);
      await handle.writeFile(bytes.subarray(0, Math.max(1, Math.floor(bytes.byteLength / 2))));
      fail("evidence_write_failed");
    }
    await handle.writeFile(contents);
    await handle.sync();
    await handle.close();
    handle = undefined;
    if (failStage === "before_rename") fail("evidence_rename_failed");
    await rename(temporary, destination);
    try {
      const directory = await open(parent, "r");
      try {
        await directory.sync();
      } finally {
        await directory.close();
      }
    } catch (error) {
      if (
        !(error instanceof Error) ||
        !("code" in error) ||
        !["EINVAL", "ENOTSUP", "EISDIR"].includes(error.code)
      ) {
        throw error;
      }
    }
  } finally {
    if (handle !== undefined) await handle.close().catch(() => undefined);
    await unlink(temporary).catch((error) => {
      if (!(error instanceof Error) || !("code" in error) || error.code !== "ENOENT") throw error;
    });
  }
}

export function argumentsFrom(argv) {
  const options = {
    baseline: DEFAULT_BASELINE,
    output: DEFAULT_OUTPUT,
    profile: "reduced",
    recordExploratory: false,
    serverResult: DEFAULT_SERVER,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--record-exploratory") {
      options.recordExploratory = true;
    } else if (["--baseline", "--output", "--profile", "--server-result"].includes(argument)) {
      const value = argv[index + 1];
      if (value === undefined) fail("usage");
      index += 1;
      if (argument === "--baseline") options.baseline = resolve(value);
      else if (argument === "--output") options.output = resolve(value);
      else if (argument === "--server-result") options.serverResult = resolve(value);
      else options.profile = value;
    } else fail("usage");
  }
  if (options.profile !== "reduced" && options.profile !== "qualified") fail("profile_invalid");
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

export async function bundledModule(entryPoint, platformName, format, globalName) {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    entryPoints: [resolve(browserRoot, entryPoint)],
    format,
    globalName,
    legalComments: "none",
    metafile: true,
    minify: true,
    platform: platformName,
    target: platformName === "browser" ? ["chrome111"] : "node20",
    treeShaking: true,
    write: false,
  });
  const output = result.outputFiles[0];
  if (output === undefined) fail("benchmark_bundle_missing");
  return Object.freeze({ inputs: Object.keys(result.metafile.inputs), source: output.text });
}

async function schemaModule() {
  const { source } = await bundledModule("benchmarks/upload-schema.ts", "node", "esm");
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

async function accountingModule() {
  const { source } = await bundledModule("benchmarks/upload-accounting.ts", "node", "esm");
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function parseCpuList(value) {
  const selected = new Set();
  for (const part of value.trim().split(",")) {
    if (part.length === 0) continue;
    const [startText, endText = startText] = part.split("-", 2);
    const start = Number(startText);
    const end = Number(endText);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end < start) {
      return 0;
    }
    for (let cpu = start; cpu <= end; cpu += 1) selected.add(cpu);
  }
  return selected.size;
}

async function selectedCpuCount() {
  try {
    const status = await readFile("/proc/self/status", "utf8");
    const line = status.split("\n").find((candidate) => candidate.startsWith("Cpus_allowed_list:"));
    return line === undefined ? 0 : parseCpuList(line.slice(line.indexOf(":") + 1));
  } catch {
    return 0;
  }
}

function normalizedArchitecture() {
  return arch() === "x64" ? "x86_64" : arch();
}

async function playwrightVersion() {
  const packageValue = JSON.parse(
    await readFile(resolve(browserRoot, "node_modules/@playwright/test/package.json"), "utf8"),
  );
  return typeof packageValue.version === "string" ? packageValue.version : "unavailable";
}

export async function browserEnvironment(browser, dedicated) {
  const cpuCount = await selectedCpuCount();
  const memoryBytes = totalmem();
  const operatingSystem = platform();
  const architecture = normalizedArchitecture();
  const requirementsMet =
    operatingSystem === "linux" &&
    architecture === "x86_64" &&
    cpuCount === 8 &&
    memoryBytes >= 16 * 1024 * 1024 * 1024 &&
    dedicated;
  return Object.freeze({
    architecture,
    browser: "chromium",
    browserRevision: `${browser.version()} @ ${chromium.executablePath()}`,
    classification: requirementsMet ? "qualified" : "unqualified",
    cpuModel: cpus()[0]?.model ?? "unavailable",
    cpuThrottleRate: 4,
    dedicatedVcpusAttested: dedicated,
    extensions: false,
    kernel: `Linux ${release()}`,
    memoryBytes,
    operatingSystem,
    playwrightVersion: await playwrightVersion(),
    profile: "B1",
    qualificationRequirementsMet: requirementsMet,
    selectedCpuCount: cpuCount,
    viewport: { height: 720, width: 1280 },
    warmHttpCache: true,
  });
}

export async function measureRun(
  browser,
  artifactSource,
  workloadSource,
  { watchdogMilliseconds = 60_000 } = {},
) {
  const context = await browser.newContext({ viewport: { height: 720, width: 1280 } });
  const page = await context.newPage();
  const session = await context.newCDPSession(page);
  try {
    await session.send("Emulation.setCPUThrottlingRate", { rate: 4 });
    await page.goto("about:blank");
    await page.evaluate(async (source) => {
      const encoded = btoa(String.fromCodePoint(...new TextEncoder().encode(source)));
      const artifactNamespace = await import(`data:text/javascript;base64,${encoded}`);
      Object.defineProperty(globalThis, "SuprnovaUploadBudgetArtifact", {
        configurable: true,
        value: artifactNamespace,
      });
    }, artifactSource);
    await page.addScriptTag({ content: workloadSource });
    const measurement = page.evaluate(async () => {
      const benchmark = globalThis.SuprnovaUploadBudget;
      if (benchmark?.UPLOAD_BUDGET_OBSERVER_MARKER !== "suprnova-upload-budget-observer-v1") {
        throw new Error("upload_budget_observer_missing");
      }
      const artifactNamespace = Reflect.get(globalThis, "SuprnovaUploadBudgetArtifact");
      return benchmark.measureU4_16(artifactNamespace);
    });
    let watchdog;
    const deadline = new Promise((_, reject) => {
      watchdog = setTimeout(
        () => reject(new UploadBudgetRunnerError("upload_budget_browser_watchdog")),
        watchdogMilliseconds,
      );
    });
    try {
      return await Promise.race([measurement, deadline]);
    } finally {
      clearTimeout(watchdog);
    }
  } finally {
    await session.detach();
    await context.close();
  }
}

function maximum(runs, key) {
  return Math.max(...runs.map((run) => run[key]));
}

function maximumTransferChunkDistribution(runs, currentRun) {
  const currentBySlot = new Map(
    currentRun.chunkBuffersByTransfer.map((transfer) => [transfer.slot, transfer]),
  );
  const bySlot = new Map();
  for (const run of runs) {
    for (const transfer of run.chunkBuffersByTransfer) {
      const prior = bySlot.get(transfer.slot);
      bySlot.set(
        transfer.slot,
        Object.freeze({
          currentBytes: currentBySlot.get(transfer.slot)?.currentBytes ?? 0,
          currentManagerBuffers: currentBySlot.get(transfer.slot)?.currentManagerBuffers ?? 0,
          currentTotalBuffers: currentBySlot.get(transfer.slot)?.currentTotalBuffers ?? 0,
          currentTransportBuffers: currentBySlot.get(transfer.slot)?.currentTransportBuffers ?? 0,
          managerHighWater: Math.max(prior?.managerHighWater ?? 0, transfer.managerHighWater),
          managerHighWaterBytes: Math.max(
            prior?.managerHighWaterBytes ?? 0,
            transfer.managerHighWaterBytes,
          ),
          totalHighWater: Math.max(prior?.totalHighWater ?? 0, transfer.totalHighWater),
          totalHighWaterBytes: Math.max(
            prior?.totalHighWaterBytes ?? 0,
            transfer.totalHighWaterBytes,
          ),
          transportHighWater: Math.max(prior?.transportHighWater ?? 0, transfer.transportHighWater),
          transportHighWaterBytes: Math.max(
            prior?.transportHighWaterBytes ?? 0,
            transfer.transportHighWaterBytes,
          ),
          slot: transfer.slot,
        }),
      );
    }
  }
  return Object.freeze([...bySlot.values()].sort((left, right) => left.slot - right.slot));
}

async function main() {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    await buildRuntimeAssets();
    const artifactPath = resolve(browserRoot, "dist/suprnova-live.uploads.esm.js");
    const artifact = await readFile(artifactPath);
    if (artifact.includes(OBSERVER_MARKER)) fail("benchmark_observer_in_production_artifact");
    const workloadBundle = await bundledModule(
      "benchmarks/upload-workloads.ts",
      "browser",
      "iife",
      "SuprnovaUploadBudget",
    );
    const workloadSource = workloadBundle.source;
    if (!workloadSource.includes(OBSERVER_MARKER)) fail("benchmark_observer_bundle_invalid");
    const accounting = await accountingModule();
    try {
      accounting.assertUploadBenchmarkBundleInputs(workloadBundle.inputs);
    } catch {
      fail("benchmark_bundle_contains_production_implementation");
    }
    const runsRequired = options.profile === "qualified" ? 3 : 1;
    const dedicated = process.env.SUPRNOVA_LIVE_B1_DEDICATED === "1";
    let environment;
    const runs = [];
    for (let index = 0; index < runsRequired; index += 1) {
      const browser = await chromium.launch({ headless: true });
      try {
        const runEnvironment = await browserEnvironment(browser, dedicated);
        if (environment === undefined) environment = runEnvironment;
        else if (JSON.stringify(environment) !== JSON.stringify(runEnvironment)) {
          fail("browser_environment_changed_between_runs");
        }
        runs.push(await measureRun(browser, artifact.toString("utf8"), workloadSource));
      } finally {
        await browser.close();
      }
    }
    const progressSamples = runs.flatMap((run) => run.progressDurationsMilliseconds);
    const schema = await schemaModule();
    const progress = schema.summarizeUploadSamples(progressSamples);
    const artifactEvidence = {
      brotliBytes: brotliCompressSync(artifact, {
        params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
      }).byteLength,
      file: "suprnova-live.uploads.esm.js",
      role: "uploads-esm",
      sha256: createHash("sha256").update(artifact).digest("hex"),
    };
    const workload = {
      activeTransfers: 4,
      chunkBytes: 256 * 1024,
      fileBytes: 16 * 1024 * 1024,
      files: 4,
    };
    const managerHighWater = runs.reduce((selected, run) =>
      run.managerOwnedBytes > selected.managerOwnedBytes ? run : selected,
    );
    const chunkHighWater = runs.reduce((selected, run) =>
      run.liveChunkBuffers >= selected.liveChunkBuffers ? run : selected,
    );
    const browserEvidence = {
      bounds: {
        maxChunksPerActiveTransfer: 2,
        maxManagerOwnedBytes: 256 * 1024,
        maxProgressP95Milliseconds: 16,
      },
      environment,
      measurements: {
        activeTransfers: 4,
        chunkBuffersByTransfer: maximumTransferChunkDistribution(runs, chunkHighWater),
        liveChunkBuffers: chunkHighWater.liveChunkBuffers,
        managerChunkBuffers: chunkHighWater.managerChunkBuffers,
        managerOwnedBytes: maximum(runs, "managerOwnedBytes"),
        managerOwnedCategories: managerHighWater.managerOwnedCategories,
        maxActiveManagerTransfers: maximum(runs, "maxActiveManagerTransfers"),
        maxChunksPerTransfer: maximum(runs, "maxChunksPerTransfer"),
        maxQueueDepth: maximum(runs, "maxQueueDepth"),
        maxSimultaneousTransportOperations: maximum(runs, "maxSimultaneousTransportOperations"),
        maxSimultaneousTransportTransfers: maximum(runs, "maxSimultaneousTransportTransfers"),
        progressP50Milliseconds: progress.p50,
        progressP95Milliseconds: progress.p95,
        retainedBytes: maximum(runs, "retainedBytes"),
        slicedBytes: runs[0].slicedBytes,
        slices: runs[0].slices,
        transportChunkBuffers: chunkHighWater.transportChunkBuffers,
      },
      methodology: {
        independentRuns: runsRequired,
        measuredSamples: progressSamples.length,
        regressionReference: "median_run_p95_v1",
        warmupIterations: 5,
      },
      runs: runs.map((run, index) => ({
        artifactSha256: artifactEvidence.sha256,
        environment,
        measurements: {
          activeTransfers: run.activeTransfers,
          chunkBuffersByTransfer: run.chunkBuffersByTransfer,
          liveChunkBuffers: run.liveChunkBuffers,
          managerChunkBuffers: run.managerChunkBuffers,
          managerOwnedBytes: run.managerOwnedBytes,
          managerOwnedCategories: run.managerOwnedCategories,
          maxActiveManagerTransfers: run.maxActiveManagerTransfers,
          maxChunksPerTransfer: run.maxChunksPerTransfer,
          maxQueueDepth: run.maxQueueDepth,
          maxSimultaneousTransportOperations: run.maxSimultaneousTransportOperations,
          maxSimultaneousTransportTransfers: run.maxSimultaneousTransportTransfers,
          progressDurationsMilliseconds: run.progressDurationsMilliseconds,
          progressP50Milliseconds: run.progressP50Milliseconds,
          progressP95Milliseconds: run.progressP95Milliseconds,
          retainedBytes: run.retainedBytes,
          slicedBytes: run.slicedBytes,
          slices: run.slices,
          transportChunkBuffers: run.transportChunkBuffers,
        },
        methodology: {
          measuredSamples: run.progressSamples,
          warmupIterations: 5,
        },
        runIndex: index + 1,
        workload,
      })),
      workload,
    };
    if (process.env.SUPRNOVA_LIVE_UPLOAD_BUDGET_DEBUG === "1") {
      process.stdout.write(
        `${JSON.stringify({ measurements: browserEvidence.measurements, runs: browserEvidence.runs.map(({ measurements }) => ({ ...measurements, progressDurationsMilliseconds: `[${String(measurements.progressDurationsMilliseconds.length)} samples]` })) }, null, 2)}\n`,
      );
    }
    const serverEvidence = schema.uploadServerEvidenceFromProcessRuns(
      await boundedJson(options.serverResult),
      artifactEvidence.sha256,
      options.profile,
    );
    const result = schema.validateUploadBudgetEvidence({
      artifact: artifactEvidence,
      browser: browserEvidence,
      recordedAt: new Date().toISOString(),
      schemaVersion: 1,
      server: serverEvidence,
      workload: "U4/16",
    });
    const baselineValue = await boundedJson(options.baseline, true);
    let baseline;
    if (options.recordExploratory) {
      const qualifiedBaseline =
        baselineValue !== null &&
        typeof baselineValue === "object" &&
        baselineValue !== null &&
        "qualifiedBaseline" in baselineValue
          ? baselineValue.qualifiedBaseline
          : null;
      if (qualifiedBaseline !== null) {
        const qualified = schema.validateUploadBudgetEvidence(qualifiedBaseline);
        if (
          qualified.browser.environment.classification !== "qualified" ||
          qualified.server.environment.classification !== "qualified"
        ) {
          fail("qualified_baseline_invalid");
        }
      }
      baseline = schema.validateUploadBudgetBaseline({
        exploratoryReference: result,
        qualifiedBaseline,
        schemaVersion: 1,
        workload: "U4/16",
      });
    } else {
      baseline = baselineValue === null ? null : schema.validateUploadBudgetBaseline(baselineValue);
    }
    const evaluation = schema.evaluateUploadBudget(
      result,
      options.profile === "qualified" ? (baseline?.qualifiedBaseline ?? null) : null,
      { artifactSha256: result.artifact.sha256, release: options.profile === "qualified" },
    );
    await atomicWriteEvidence(
      options.output,
      `${JSON.stringify(result, null, 2)}\n`,
      options.baseline,
    );
    if (options.recordExploratory && evaluation.issues.length === 0 && baseline !== null) {
      await atomicWriteEvidence(options.baseline, `${JSON.stringify(baseline, null, 2)}\n`, null);
    }
    process.stdout.write(
      `U4/16 upload budget classification=${evaluation.classification} browser_p50=${String(result.browser.measurements.progressP50Milliseconds)}ms browser_p95=${String(result.browser.measurements.progressP95Milliseconds)}ms server_p50=${String(result.server.measurements.p50Microseconds)}us server_p95=${String(result.server.measurements.p95Microseconds)}us chunks=${String(result.browser.measurements.liveChunkBuffers)} manager=${String(result.browser.measurements.managerOwnedBytes)}B artifact=${result.artifact.sha256} output=${options.output}\n`,
    );
    if (evaluation.issues.length > 0) {
      process.stderr.write(`U4/16 upload budget failed: ${evaluation.issues.join(",")}\n`);
      process.exitCode = 1;
    }
    if (evaluation.observations.length > 0) {
      process.stdout.write(
        `U4/16 upload budget observations: ${evaluation.observations.join(",")}\n`,
      );
    }
  } catch (error) {
    const code = error instanceof UploadBudgetRunnerError ? error.code : "internal";
    if (code === "internal" && error instanceof Error) process.stderr.write(`${error.stack}\n`);
    process.stderr.write(`U4/16 upload budget runner failed: ${code}\n`);
    process.exitCode = code === "usage" ? 64 : 1;
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
