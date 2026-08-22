import { classifyP95Regression, summarizeSamples, type SampleSummary } from "./statistics.js";

export const BROWSER_BUDGET_LIMITS = Object.freeze({
  coreBrotliBytes: 45 * 1024,
  d100ConnectP95Ms: 50,
  idleMainThreadMs: 5,
  coreMutationObservers: 1,
  idleNetworkRequests: 0,
  idlePollingOperations: 0,
  retainedBytesPerIsland: 12 * 1024,
  m1kMorphP95Ms: 32,
  m5kMorphP95Ms: 100,
});

export interface BenchmarkEnvironment {
  readonly platform: string;
  readonly architecture: string;
  readonly kernel: string;
  readonly cpuModel: string;
  readonly logicalCpuCount: number;
  readonly memoryBytes: number;
  readonly cpuGovernor: string;
  readonly dedicated: boolean;
  readonly loopback: boolean;
  readonly playwrightVersion: string;
  readonly browserName: "chromium";
  readonly browserVersion: string;
  readonly browserRevision: string;
  readonly viewport: Readonly<{ width: number; height: number }>;
  readonly cpuThrottleRate: number;
  readonly extensions: boolean;
  readonly warmHttpCache: boolean;
}

export interface BrowserBudgetResult {
  readonly schemaVersion: 1;
  readonly classification: "exploratory" | "b1";
  readonly recordedAt: string;
  readonly artifact: Readonly<{
    file: "suprnova-live.esm.js";
    sha256: string;
    brotliBytes: number;
  }>;
  readonly environment: BenchmarkEnvironment;
  readonly methodology: Readonly<{
    warmupSamples: number;
    measuredSamples: number;
    independentRuns: number;
    idleDurationMs: number;
    retainedMemory: "d100-minus-control-minus-fixed-runtime-v1";
    mainThreadTime: "cdp-performance-task-duration-v1";
    observerCount: "instrumented-runtime-observer-factory-v1";
    morphMeasurement: "bundled-production-morph-port-v1";
    morphDeadlineMs: 10_000;
    correctnessEnabled: true;
    accessibilityEnabled: true;
    lifecycleEnabled: true;
  }>;
  readonly workloads: Readonly<{
    D100: Readonly<{
      documentBytes: 65_536;
      islandCount: 100;
      connect: SampleSummary;
      idleMainThreadMs: number;
      coreMutationObservers: number;
      idleNetworkRequests: number;
      idlePollingOperations: number;
      retainedBytesPerIsland: number;
    }>;
    M1K: Readonly<{
      elementCount: 1_000;
      maximumDepth: 12;
      changedNodeCount: 100;
      morph: SampleSummary;
    }>;
    M5K: Readonly<{
      elementCount: 5_000;
      maximumDepth: 24;
      changedNodeCount: 500;
      morph: SampleSummary;
    }>;
  }>;
  readonly independentP95Ms: Readonly<{
    d100Connect: readonly number[];
    m1kMorph: readonly number[];
    m5kMorph: readonly number[];
  }>;
}

export interface BudgetEvaluation {
  readonly status: "pass" | "failed" | "unqualified";
  readonly codes: readonly string[];
  readonly regressions: Readonly<Record<string, ReturnType<typeof classifyP95Regression>>>;
}

const SHA256 = /^[a-f0-9]{64}$/u;
const VERSION = /^[0-9]+(?:\.[0-9]+){0,3}$/u;

function fail(code: string): never {
  throw new Error(code);
}

function record(value: unknown, code: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail(code);
  return value as Record<string, unknown>;
}

function exactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  code: string,
): void {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length ||
    actual.some((key, index) => key !== sortedExpected[index])
  ) {
    fail(code);
  }
}

function text(value: unknown, code: string, maximum = 256): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maximum ||
    value.trim() !== value
  ) {
    fail(code);
  }
  return value;
}

function finite(value: unknown, code: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) fail(code);
  return value;
}

function positiveInteger(value: unknown, code: string, maximum = Number.MAX_SAFE_INTEGER): number {
  if (!Number.isSafeInteger(value) || (value as number) <= 0 || (value as number) > maximum) {
    fail(code);
  }
  return value as number;
}

function boolean(value: unknown, code: string): boolean {
  if (typeof value !== "boolean") fail(code);
  return value;
}

function numberArray(value: unknown, code: string, maximum = 100): readonly number[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > maximum) fail(code);
  const result = value.map((candidate) => finite(candidate, code));
  return Object.freeze(result);
}

function summary(value: unknown, code: string): SampleSummary {
  const candidate = record(value, code);
  exactKeys(candidate, ["samplesMs", "sampleCount", "p50Ms", "p95Ms"], code);
  const samples = numberArray(candidate["samplesMs"], code);
  const calculated = summarizeSamples(samples);
  if (
    candidate["sampleCount"] !== calculated.sampleCount ||
    candidate["p50Ms"] !== calculated.p50Ms ||
    candidate["p95Ms"] !== calculated.p95Ms
  ) {
    fail(code);
  }
  return calculated;
}

function validateEnvironment(value: unknown): BenchmarkEnvironment {
  const candidate = record(value, "environment_invalid");
  exactKeys(
    candidate,
    [
      "platform",
      "architecture",
      "kernel",
      "cpuModel",
      "logicalCpuCount",
      "memoryBytes",
      "cpuGovernor",
      "dedicated",
      "loopback",
      "playwrightVersion",
      "browserName",
      "browserVersion",
      "browserRevision",
      "viewport",
      "cpuThrottleRate",
      "extensions",
      "warmHttpCache",
    ],
    "environment_invalid",
  );
  const viewport = record(candidate["viewport"], "environment_invalid");
  exactKeys(viewport, ["width", "height"], "environment_invalid");
  const browserName = candidate["browserName"];
  if (browserName !== "chromium") fail("environment_browser_invalid");
  const browserVersion = text(candidate["browserVersion"], "environment_browser_invalid", 64);
  if (!VERSION.test(browserVersion)) fail("environment_browser_invalid");
  return Object.freeze({
    platform: text(candidate["platform"], "environment_invalid", 32),
    architecture: text(candidate["architecture"], "environment_invalid", 32),
    kernel: text(candidate["kernel"], "environment_invalid", 128),
    cpuModel: text(candidate["cpuModel"], "environment_invalid", 256),
    logicalCpuCount: positiveInteger(candidate["logicalCpuCount"], "environment_invalid", 4_096),
    memoryBytes: positiveInteger(candidate["memoryBytes"], "environment_invalid"),
    cpuGovernor: text(candidate["cpuGovernor"], "environment_invalid", 64),
    dedicated: boolean(candidate["dedicated"], "environment_invalid"),
    loopback: boolean(candidate["loopback"], "environment_invalid"),
    playwrightVersion: text(candidate["playwrightVersion"], "environment_invalid", 32),
    browserName,
    browserVersion,
    browserRevision: text(candidate["browserRevision"], "environment_invalid", 64),
    viewport: Object.freeze({
      width: positiveInteger(viewport["width"], "environment_invalid", 16_384),
      height: positiveInteger(viewport["height"], "environment_invalid", 16_384),
    }),
    cpuThrottleRate: finite(candidate["cpuThrottleRate"], "environment_invalid"),
    extensions: boolean(candidate["extensions"], "environment_invalid"),
    warmHttpCache: boolean(candidate["warmHttpCache"], "environment_invalid"),
  });
}

export function classifyBenchmarkEnvironment(
  environment: BenchmarkEnvironment,
): "exploratory" | "b1" {
  return environment.platform === "linux" &&
    environment.architecture === "x64" &&
    environment.logicalCpuCount === 8 &&
    environment.memoryBytes === 16 * 1024 ** 3 &&
    environment.cpuGovernor === "performance" &&
    environment.dedicated &&
    environment.loopback &&
    environment.playwrightVersion === "1.62.1" &&
    environment.viewport.width === 1280 &&
    environment.viewport.height === 720 &&
    environment.cpuThrottleRate === 4 &&
    !environment.extensions &&
    environment.warmHttpCache
    ? "b1"
    : "exploratory";
}

function validateMethodology(value: unknown) {
  const candidate = record(value, "methodology_invalid");
  exactKeys(
    candidate,
    [
      "warmupSamples",
      "measuredSamples",
      "independentRuns",
      "idleDurationMs",
      "retainedMemory",
      "mainThreadTime",
      "observerCount",
      "morphMeasurement",
      "morphDeadlineMs",
      "correctnessEnabled",
      "accessibilityEnabled",
      "lifecycleEnabled",
    ],
    "methodology_invalid",
  );
  if (
    candidate["retainedMemory"] !== "d100-minus-control-minus-fixed-runtime-v1" ||
    candidate["mainThreadTime"] !== "cdp-performance-task-duration-v1" ||
    candidate["observerCount"] !== "instrumented-runtime-observer-factory-v1" ||
    candidate["morphMeasurement"] !== "bundled-production-morph-port-v1" ||
    candidate["morphDeadlineMs"] !== 10_000 ||
    candidate["correctnessEnabled"] !== true ||
    candidate["accessibilityEnabled"] !== true ||
    candidate["lifecycleEnabled"] !== true
  ) {
    fail("methodology_invalid");
  }
  const independentRuns = positiveInteger(candidate["independentRuns"], "methodology_invalid", 3);
  return Object.freeze({
    warmupSamples: positiveInteger(candidate["warmupSamples"], "methodology_invalid", 100),
    measuredSamples: positiveInteger(candidate["measuredSamples"], "methodology_invalid", 100),
    independentRuns,
    idleDurationMs: positiveInteger(candidate["idleDurationMs"], "methodology_invalid", 120_000),
    retainedMemory: "d100-minus-control-minus-fixed-runtime-v1" as const,
    mainThreadTime: "cdp-performance-task-duration-v1" as const,
    observerCount: "instrumented-runtime-observer-factory-v1" as const,
    morphMeasurement: "bundled-production-morph-port-v1" as const,
    morphDeadlineMs: 10_000 as const,
    correctnessEnabled: true as const,
    accessibilityEnabled: true as const,
    lifecycleEnabled: true as const,
  });
}

function validateD100(value: unknown) {
  const candidate = record(value, "d100_invalid");
  exactKeys(
    candidate,
    [
      "documentBytes",
      "islandCount",
      "connect",
      "idleMainThreadMs",
      "coreMutationObservers",
      "idleNetworkRequests",
      "idlePollingOperations",
      "retainedBytesPerIsland",
    ],
    "d100_invalid",
  );
  if (candidate["documentBytes"] !== 65_536 || candidate["islandCount"] !== 100) {
    fail("d100_shape_invalid");
  }
  return Object.freeze({
    documentBytes: 65_536 as const,
    islandCount: 100 as const,
    connect: summary(candidate["connect"], "d100_connect_invalid"),
    idleMainThreadMs: finite(candidate["idleMainThreadMs"], "d100_invalid"),
    coreMutationObservers: finite(candidate["coreMutationObservers"], "d100_invalid"),
    idleNetworkRequests: finite(candidate["idleNetworkRequests"], "d100_invalid"),
    idlePollingOperations: finite(candidate["idlePollingOperations"], "d100_invalid"),
    retainedBytesPerIsland: finite(candidate["retainedBytesPerIsland"], "d100_invalid"),
  });
}

function validateMorph(value: unknown, id: "M1K"): BrowserBudgetResult["workloads"]["M1K"];
function validateMorph(value: unknown, id: "M5K"): BrowserBudgetResult["workloads"]["M5K"];
function validateMorph(value: unknown, id: "M1K" | "M5K") {
  const candidate = record(value, `${id.toLowerCase()}_invalid`);
  exactKeys(
    candidate,
    ["elementCount", "maximumDepth", "changedNodeCount", "morph"],
    `${id.toLowerCase()}_invalid`,
  );
  const expected =
    id === "M1K"
      ? { elementCount: 1_000 as const, maximumDepth: 12 as const, changedNodeCount: 100 as const }
      : { elementCount: 5_000 as const, maximumDepth: 24 as const, changedNodeCount: 500 as const };
  if (
    candidate["elementCount"] !== expected.elementCount ||
    candidate["maximumDepth"] !== expected.maximumDepth ||
    candidate["changedNodeCount"] !== expected.changedNodeCount
  ) {
    fail(`${id.toLowerCase()}_shape_invalid`);
  }
  return Object.freeze({
    ...expected,
    morph: summary(candidate["morph"], `${id.toLowerCase()}_morph_invalid`),
  });
}

export function validateBrowserBudgetResult(value: unknown): BrowserBudgetResult {
  const candidate = record(value, "browser_budget_invalid");
  exactKeys(
    candidate,
    [
      "schemaVersion",
      "classification",
      "recordedAt",
      "artifact",
      "environment",
      "methodology",
      "workloads",
      "independentP95Ms",
    ],
    "browser_budget_invalid",
  );
  if (candidate["schemaVersion"] !== 1) fail("browser_budget_version");
  const recordedAt = text(candidate["recordedAt"], "recorded_at_invalid", 40);
  if (new Date(recordedAt).toISOString() !== recordedAt) fail("recorded_at_invalid");

  const artifact = record(candidate["artifact"], "artifact_invalid");
  exactKeys(artifact, ["file", "sha256", "brotliBytes"], "artifact_invalid");
  if (artifact["file"] !== "suprnova-live.esm.js" || !SHA256.test(String(artifact["sha256"]))) {
    fail("artifact_invalid");
  }
  const environment = validateEnvironment(candidate["environment"]);
  const classification = classifyBenchmarkEnvironment(environment);
  if (candidate["classification"] !== classification) fail("classification_invalid");
  const methodology = validateMethodology(candidate["methodology"]);
  const workloads = record(candidate["workloads"], "workloads_invalid");
  exactKeys(workloads, ["D100", "M1K", "M5K"], "workloads_invalid");
  const D100 = validateD100(workloads["D100"]);
  const M1K = validateMorph(workloads["M1K"], "M1K");
  const M5K = validateMorph(workloads["M5K"], "M5K");
  for (const item of [D100.connect, M1K.morph, M5K.morph]) {
    if (classification === "b1" && item.sampleCount < 30) fail("sample_count_b1");
    if (item.sampleCount < methodology.measuredSamples) fail("sample_count_methodology");
  }
  if (classification === "b1" && methodology.idleDurationMs !== 30_000) {
    fail("idle_duration_b1");
  }

  const independent = record(candidate["independentP95Ms"], "independent_runs_invalid");
  exactKeys(independent, ["d100Connect", "m1kMorph", "m5kMorph"], "independent_runs_invalid");
  const d100Connect = numberArray(independent["d100Connect"], "independent_runs_invalid", 3);
  const m1kMorph = numberArray(independent["m1kMorph"], "independent_runs_invalid", 3);
  const m5kMorph = numberArray(independent["m5kMorph"], "independent_runs_invalid", 3);
  if (
    d100Connect.length !== methodology.independentRuns ||
    m1kMorph.length !== methodology.independentRuns ||
    m5kMorph.length !== methodology.independentRuns
  ) {
    fail("independent_runs_invalid");
  }

  return Object.freeze({
    schemaVersion: 1,
    classification,
    recordedAt,
    artifact: Object.freeze({
      file: "suprnova-live.esm.js" as const,
      sha256: artifact["sha256"] as string,
      brotliBytes: positiveInteger(artifact["brotliBytes"], "artifact_invalid", 4_194_304),
    }),
    environment,
    methodology,
    workloads: Object.freeze({ D100, M1K, M5K }),
    independentP95Ms: Object.freeze({ d100Connect, m1kMorph, m5kMorph }),
  });
}

function sameEnvironment(left: BenchmarkEnvironment, right: BenchmarkEnvironment): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function evaluateBrowserBudget(
  result: BrowserBudgetResult,
  baseline: BrowserBudgetResult | undefined,
  options: Readonly<{ release: boolean }>,
): BudgetEvaluation {
  const codes: string[] = [];
  const regressions: Record<string, ReturnType<typeof classifyP95Regression>> = {};
  let status: BudgetEvaluation["status"] = "pass";
  if (result.artifact.brotliBytes > BROWSER_BUDGET_LIMITS.coreBrotliBytes) {
    codes.push("core_transfer_exceeded");
    status = "failed";
  }
  if (options.release && result.classification !== "b1") {
    return Object.freeze({
      status: "unqualified",
      codes: Object.freeze(["b1_required"]),
      regressions: Object.freeze(regressions),
    });
  }
  if (result.classification === "b1") {
    const caps = [
      [
        result.workloads.D100.connect.p95Ms,
        BROWSER_BUDGET_LIMITS.d100ConnectP95Ms,
        "d100_connect_exceeded",
      ],
      [
        result.workloads.D100.idleMainThreadMs,
        BROWSER_BUDGET_LIMITS.idleMainThreadMs,
        "idle_main_thread_exceeded",
      ],
      [
        result.workloads.D100.coreMutationObservers,
        BROWSER_BUDGET_LIMITS.coreMutationObservers,
        "observer_count_exceeded",
      ],
      [
        result.workloads.D100.idleNetworkRequests,
        BROWSER_BUDGET_LIMITS.idleNetworkRequests,
        "idle_network_detected",
      ],
      [
        result.workloads.D100.idlePollingOperations,
        BROWSER_BUDGET_LIMITS.idlePollingOperations,
        "idle_polling_detected",
      ],
      [
        result.workloads.D100.retainedBytesPerIsland,
        BROWSER_BUDGET_LIMITS.retainedBytesPerIsland,
        "retained_memory_exceeded",
      ],
      [result.workloads.M1K.morph.p95Ms, BROWSER_BUDGET_LIMITS.m1kMorphP95Ms, "m1k_morph_exceeded"],
      [result.workloads.M5K.morph.p95Ms, BROWSER_BUDGET_LIMITS.m5kMorphP95Ms, "m5k_morph_exceeded"],
    ] as const;
    for (const [actual, limit, code] of caps) {
      if (actual > limit) {
        codes.push(code);
        status = "failed";
      }
    }
  }
  if (baseline !== undefined && sameEnvironment(result.environment, baseline.environment)) {
    const comparisons = [
      ["d100Connect", baseline.workloads.D100.connect.p95Ms, result.independentP95Ms.d100Connect],
      ["m1kMorph", baseline.workloads.M1K.morph.p95Ms, result.independentP95Ms.m1kMorph],
      ["m5kMorph", baseline.workloads.M5K.morph.p95Ms, result.independentP95Ms.m5kMorph],
    ] as const;
    for (const [name, baselineP95, runs] of comparisons) {
      const regression = classifyP95Regression(baselineP95, runs);
      regressions[name] = regression;
      if (regression.state === "confirmed") {
        codes.push(`${name}_regression_confirmed`);
        status = "failed";
      } else if (regression.state === "candidate" && status === "pass") {
        codes.push(`${name}_regression_confirmation_required`);
        status = "unqualified";
      }
    }
  }
  return Object.freeze({
    status,
    codes: Object.freeze(codes),
    regressions: Object.freeze(regressions),
  });
}
