import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { arch, cpus, platform, release, totalmem } from "node:os";
import { dirname, resolve } from "node:path";
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

class UploadBudgetRunnerError extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}

function fail(code) {
  throw new UploadBudgetRunnerError(code);
}

export function argumentsFrom(argv) {
  const options = {
    baseline: DEFAULT_BASELINE,
    output: DEFAULT_OUTPUT,
    profile: "reduced",
    serverResult: DEFAULT_SERVER,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (["--baseline", "--output", "--profile", "--server-result"].includes(argument)) {
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

async function bundledModule(entryPoint, platformName, format, globalName) {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    entryPoints: [resolve(browserRoot, entryPoint)],
    format,
    globalName,
    legalComments: "none",
    minify: true,
    platform: platformName,
    target: platformName === "browser" ? ["chrome111"] : "node20",
    treeShaking: true,
    write: false,
  });
  const output = result.outputFiles[0];
  if (output === undefined) fail("benchmark_bundle_missing");
  return output.text;
}

async function schemaModule() {
  const source = await bundledModule("benchmarks/upload-schema.ts", "node", "esm");
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

async function browserEnvironment(browser, dedicated) {
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

async function measureRun(browser, artifactSource, workloadSource) {
  const context = await browser.newContext({ viewport: { height: 720, width: 1280 } });
  const page = await context.newPage();
  const session = await context.newCDPSession(page);
  try {
    await session.send("Emulation.setCPUThrottlingRate", { rate: 4 });
    await page.goto("about:blank");
    await page.evaluate(async (source) => {
      const encoded = btoa(String.fromCodePoint(...new TextEncoder().encode(source)));
      await import(`data:text/javascript;base64,${encoded}`);
    }, artifactSource);
    await page.addScriptTag({ content: workloadSource });
    return await page.evaluate(async () => {
      const benchmark = globalThis.SuprnovaUploadBudget;
      if (benchmark?.UPLOAD_BUDGET_OBSERVER_MARKER !== "suprnova-upload-budget-observer-v1") {
        throw new Error("upload_budget_observer_missing");
      }
      return benchmark.measureU4_16();
    });
  } finally {
    await session.detach();
    await context.close();
  }
}

function maximum(runs, key) {
  return Math.max(...runs.map((run) => run[key]));
}

async function main() {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    await buildRuntimeAssets();
    const artifactPath = resolve(browserRoot, "dist/suprnova-live.uploads.esm.js");
    const artifact = await readFile(artifactPath);
    if (artifact.includes(OBSERVER_MARKER)) fail("benchmark_observer_in_production_artifact");
    const workloadSource = await bundledModule(
      "benchmarks/upload-workloads.ts",
      "browser",
      "iife",
      "SuprnovaUploadBudget",
    );
    if (!workloadSource.includes(OBSERVER_MARKER)) fail("benchmark_observer_bundle_invalid");
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
    const browserEvidence = {
      bounds: {
        maxChunksPerActiveTransfer: 2,
        maxManagerOwnedBytes: 256 * 1024,
        maxProgressP95Milliseconds: 16,
      },
      environment,
      measurements: {
        activeTransfers: 4,
        liveChunkBuffers: maximum(runs, "liveChunkBuffers"),
        managerOwnedBytes: maximum(runs, "managerOwnedBytes"),
        maxChunksPerTransfer: maximum(runs, "maxChunksPerTransfer"),
        maxConcurrentTransfers: maximum(runs, "maxConcurrentTransfers"),
        maxQueueDepth: maximum(runs, "maxQueueDepth"),
        progressP50Milliseconds: progress.p50,
        progressP95Milliseconds: progress.p95,
        retainedBytes: maximum(runs, "retainedBytes"),
        slicedBytes: runs[0].slicedBytes,
        slices: runs[0].slices,
      },
      methodology: {
        independentRuns: runsRequired,
        measuredSamples: progressSamples.length,
        warmupIterations: 5,
      },
      workload: {
        activeTransfers: 4,
        chunkBytes: 256 * 1024,
        fileBytes: 16 * 1024 * 1024,
        files: 4,
      },
    };
    const result = schema.validateUploadBudgetEvidence({
      artifact: {
        brotliBytes: brotliCompressSync(artifact, {
          params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
        }).byteLength,
        file: "suprnova-live.uploads.esm.js",
        role: "uploads-esm",
        sha256: createHash("sha256").update(artifact).digest("hex"),
      },
      browser: browserEvidence,
      recordedAt: new Date().toISOString(),
      schemaVersion: 1,
      server: await boundedJson(options.serverResult),
      workload: "U4/16",
    });
    const baselineValue = await boundedJson(options.baseline, true);
    const baseline =
      baselineValue === null ? null : schema.validateUploadBudgetBaseline(baselineValue);
    const evaluation = schema.evaluateUploadBudget(
      result,
      options.profile === "qualified" ? (baseline?.qualifiedBaseline ?? null) : null,
      { artifactSha256: result.artifact.sha256, release: options.profile === "qualified" },
    );
    await writeFile(options.output, `${JSON.stringify(result, null, 2)}\n`, "utf8");
    process.stdout.write(
      `U4/16 upload budget classification=${evaluation.classification} browser_p50=${String(result.browser.measurements.progressP50Milliseconds)}ms browser_p95=${String(result.browser.measurements.progressP95Milliseconds)}ms server_p50=${String(result.server.measurements.p50Microseconds)}us server_p95=${String(result.server.measurements.p95Microseconds)}us chunks=${String(result.browser.measurements.liveChunkBuffers)} manager=${String(result.browser.measurements.managerOwnedBytes)}B artifact=${result.artifact.sha256} output=${options.output}\n`,
    );
    if (evaluation.issues.length > 0) {
      process.stderr.write(`U4/16 upload budget failed: ${evaluation.issues.join(",")}\n`);
      process.exitCode = 1;
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
