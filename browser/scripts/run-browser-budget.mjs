import { spawnSync } from "node:child_process";
import { lstat, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";

import { buildRuntimeAssets } from "./build.mjs";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_OUTPUT = resolve(browserRoot, "benchmarks/local/latest.json");
const DEFAULT_BASELINE = resolve(browserRoot, "benchmarks/baselines/browser-budget-v1.json");
const MAX_RESULT_BYTES = 1_048_576;

class BrowserBudgetRunnerError extends Error {
  constructor(code) {
    super(code);
    this.name = "BrowserBudgetRunnerError";
    this.code = code;
  }
}

function fail(code) {
  throw new BrowserBudgetRunnerError(code);
}

function positiveInteger(value, maximum, code) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0 || parsed > maximum) fail(code);
  return parsed;
}

export function argumentsFrom(argv) {
  const options = {
    baseline: DEFAULT_BASELINE,
    dedicated: false,
    idleMs: 30_000,
    output: DEFAULT_OUTPUT,
    release: false,
    runs: 1,
    samples: 30,
    warmups: 5,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--dedicated") options.dedicated = true;
    else if (argument === "--release") options.release = true;
    else if (
      ["--baseline", "--idle-ms", "--output", "--runs", "--samples", "--warmups"].includes(argument)
    ) {
      const value = argv[index + 1];
      if (value === undefined) fail("usage");
      index += 1;
      if (argument === "--baseline") options.baseline = resolve(value);
      else if (argument === "--output") options.output = resolve(value);
      else if (argument === "--idle-ms")
        options.idleMs = positiveInteger(value, 120_000, "idle_invalid");
      else if (argument === "--runs") options.runs = positiveInteger(value, 3, "runs_invalid");
      else if (argument === "--samples")
        options.samples = positiveInteger(value, 100, "samples_invalid");
      else options.warmups = positiveInteger(value, 100, "warmups_invalid");
    } else fail("usage");
  }
  if (options.release && (options.samples < 30 || options.idleMs !== 30_000)) {
    fail("release_methodology_invalid");
  }
  if (options.output === options.baseline) fail("baseline_overwrite_forbidden");
  return Object.freeze(options);
}

async function boundedJson(path, missingAllowed) {
  let metadata;
  try {
    metadata = await lstat(path);
  } catch (error) {
    if (missingAllowed && error instanceof Error && "code" in error && error.code === "ENOENT") {
      return null;
    }
    fail("result_unreadable");
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > MAX_RESULT_BYTES) {
    fail("result_unreadable");
  }
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    fail("result_invalid");
  }
}

async function schemaModule() {
  const result = await build({
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
  const output = result.outputFiles[0];
  if (output === undefined) fail("schema_build_failed");
  return import(`data:text/javascript;base64,${Buffer.from(output.contents).toString("base64")}`);
}

function runPlaywright(options) {
  const cli = resolve(browserRoot, "node_modules/@playwright/test/cli.js");
  const execution = spawnSync(
    process.execPath,
    [cli, "test", "e2e/performance.spec.ts", "--project=chromium", "--reporter=line"],
    {
      cwd: browserRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        SUPRNOVA_BROWSER_BUDGET_OUTPUT: options.output,
        SUPRNOVA_BROWSER_BUDGET_RECORD: "1",
        SUPRNOVA_BENCHMARK_CPU_THROTTLE: "4",
        SUPRNOVA_BENCHMARK_DEDICATED: options.dedicated ? "1" : "0",
        SUPRNOVA_BENCHMARK_IDLE_MS: String(options.idleMs),
        SUPRNOVA_BENCHMARK_RUNS: String(options.runs),
        SUPRNOVA_BENCHMARK_SAMPLES: String(options.samples),
        SUPRNOVA_BENCHMARK_WARMUPS: String(options.warmups),
      },
      maxBuffer: 1_048_576,
      timeout: 20 * 60_000,
    },
  );
  if (execution.error !== undefined || execution.status !== 0) {
    const output = `${execution.stdout}${execution.stderr}`.slice(-8_192);
    process.stderr.write(output);
    fail("playwright_measurement_failed");
  }
}

async function main() {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    await buildRuntimeAssets();
    runPlaywright(options);
    const schema = await schemaModule();
    const result = schema.validateBrowserBudgetResult(await boundedJson(options.output, false));
    const baselineValue = await boundedJson(options.baseline, true);
    const baseline =
      baselineValue === null ? undefined : schema.validateBrowserBudgetResult(baselineValue);
    const evaluation = schema.evaluateBrowserBudget(result, baseline, { release: options.release });
    process.stdout.write(
      `browser budget ${evaluation.status} classification=${result.classification} d100_p95=${String(result.workloads.D100.connect.p95Ms)}ms m1k_p95=${String(result.workloads.M1K.morph.p95Ms)}ms m5k_p95=${String(result.workloads.M5K.morph.p95Ms)}ms retained=${String(result.workloads.D100.retainedBytesPerIsland)}B output=${options.output}\n`,
    );
    if (evaluation.codes.length > 0) {
      process.stdout.write(`browser budget codes=${evaluation.codes.join(",")}\n`);
    }
    if (evaluation.status === "failed") process.exitCode = 1;
    else if (evaluation.status === "unqualified") process.exitCode = 2;
  } catch (error) {
    const code = error instanceof BrowserBudgetRunnerError ? error.code : "internal";
    process.stderr.write(`browser budget runner failed: ${code}\n`);
    process.exitCode = code === "usage" ? 64 : 1;
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
