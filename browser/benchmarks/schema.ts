import { classifyP95Regression, summarizeSamples, type SampleSummary } from "./statistics.js";

export const BROWSER_BUDGET_LIMITS = Object.freeze({
  d100ConnectP95Ms: 50,
  idleMainThreadMs: 5,
  coreMutationObservers: 1,
  idleNetworkRequests: 0,
  idlePollingOperations: 0,
  retainedBytesPerIsland: 12 * 1024,
  m1kMorphP95Ms: 32,
  m5kMorphP95Ms: 100,
  e100RetainedBytesPerSubscription: 8 * 1024,
  e100QueuedEventsPerDocument: 64,
  e100QueuedBytesPerDocument: 256 * 1024,
  e100DispatchEffectP95Ms: 8,
  e100RefreshQueuedPerIsland: 1,
  e100RefreshInFlightPerIsland: 1,
  r100RetainedBytesPerIsland: 12 * 1024,
  r100OriginConcurrentHandshakes: 8,
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
  readonly asyncArtifact: Readonly<{
    file: "suprnova-live.async.esm.js";
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
    E100: Readonly<{
      subscriptionCount: 100;
      presentationEventCount: 1_000;
      eventEnvelopeBytes: 1_024;
      scheduledDurationMs: 10_000;
      refreshInvalidationCount: 100;
      physicalConnectionCount: number;
      handshakeCount: number;
      dispatchEffect: SampleSummary;
      peakRetainedAsyncBytes: number;
      retainedBytesPerSubscription: number;
      queuedEventPeak: number;
      queuedBytePeak: number;
      maximumQueuedRefreshesPerIsland: number;
      maximumInFlightRefreshesPerIsland: number;
      currentSubscriptionCount: number;
    }>;
    R100: Readonly<{
      subscriptionCount: 100;
      simultaneousContinuityLosses: 100;
      documentReconnectHandshakes: number;
      recovery: SampleSummary;
      maximumRecoverySkewMs: number;
      recoveredSubscriptionCount: number;
      currentSubscriptionCount: number;
      starvedSubscriptionCount: number;
      maximumConcurrentReauthorizations: number;
      retainedBytesPerIsland: number;
      pollingMaximumSameTick: number;
      multiDocument: Readonly<{
        documentCount: 16;
        completedHandshakes: number;
        maximumConcurrentHandshakes: number;
      }>;
    }>;
  }>;
  readonly independentP95Ms: Readonly<{
    d100Connect: readonly number[];
    m1kMorph: readonly number[];
    m5kMorph: readonly number[];
    e100DispatchEffect: readonly number[];
    r100Recovery: readonly number[];
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

function nonnegativeInteger(
  value: unknown,
  code: string,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0 || (value as number) > maximum) {
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

function summary(value: unknown, code: string, maximumSamples = 100): SampleSummary {
  const candidate = record(value, code);
  exactKeys(candidate, ["samplesMs", "sampleCount", "p50Ms", "p95Ms"], code);
  const samples = numberArray(candidate["samplesMs"], code, maximumSamples);
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

function validateE100(value: unknown): BrowserBudgetResult["workloads"]["E100"] {
  const candidate = record(value, "e100_invalid");
  exactKeys(
    candidate,
    [
      "subscriptionCount",
      "presentationEventCount",
      "eventEnvelopeBytes",
      "scheduledDurationMs",
      "refreshInvalidationCount",
      "physicalConnectionCount",
      "handshakeCount",
      "dispatchEffect",
      "peakRetainedAsyncBytes",
      "retainedBytesPerSubscription",
      "queuedEventPeak",
      "queuedBytePeak",
      "maximumQueuedRefreshesPerIsland",
      "maximumInFlightRefreshesPerIsland",
      "currentSubscriptionCount",
    ],
    "e100_invalid",
  );
  if (
    candidate["subscriptionCount"] !== 100 ||
    candidate["presentationEventCount"] !== 1_000 ||
    candidate["eventEnvelopeBytes"] !== 1_024 ||
    candidate["scheduledDurationMs"] !== 10_000 ||
    candidate["refreshInvalidationCount"] !== 100
  ) {
    fail("e100_shape_invalid");
  }
  return Object.freeze({
    subscriptionCount: 100 as const,
    presentationEventCount: 1_000 as const,
    eventEnvelopeBytes: 1_024 as const,
    scheduledDurationMs: 10_000 as const,
    refreshInvalidationCount: 100 as const,
    physicalConnectionCount: nonnegativeInteger(
      candidate["physicalConnectionCount"],
      "e100_invalid",
    ),
    handshakeCount: nonnegativeInteger(candidate["handshakeCount"], "e100_invalid"),
    dispatchEffect: summary(candidate["dispatchEffect"], "e100_dispatch_effect_invalid", 10_000),
    peakRetainedAsyncBytes: finite(candidate["peakRetainedAsyncBytes"], "e100_invalid"),
    retainedBytesPerSubscription: finite(candidate["retainedBytesPerSubscription"], "e100_invalid"),
    queuedEventPeak: nonnegativeInteger(candidate["queuedEventPeak"], "e100_invalid"),
    queuedBytePeak: nonnegativeInteger(candidate["queuedBytePeak"], "e100_invalid"),
    maximumQueuedRefreshesPerIsland: nonnegativeInteger(
      candidate["maximumQueuedRefreshesPerIsland"],
      "e100_invalid",
    ),
    maximumInFlightRefreshesPerIsland: nonnegativeInteger(
      candidate["maximumInFlightRefreshesPerIsland"],
      "e100_invalid",
    ),
    currentSubscriptionCount: nonnegativeInteger(
      candidate["currentSubscriptionCount"],
      "e100_invalid",
      100,
    ),
  });
}

function validateR100(value: unknown): BrowserBudgetResult["workloads"]["R100"] {
  const candidate = record(value, "r100_invalid");
  exactKeys(
    candidate,
    [
      "subscriptionCount",
      "simultaneousContinuityLosses",
      "documentReconnectHandshakes",
      "recovery",
      "maximumRecoverySkewMs",
      "recoveredSubscriptionCount",
      "currentSubscriptionCount",
      "starvedSubscriptionCount",
      "maximumConcurrentReauthorizations",
      "retainedBytesPerIsland",
      "pollingMaximumSameTick",
      "multiDocument",
    ],
    "r100_invalid",
  );
  if (candidate["subscriptionCount"] !== 100 || candidate["simultaneousContinuityLosses"] !== 100) {
    fail("r100_shape_invalid");
  }
  const multiDocument = record(candidate["multiDocument"], "r100_invalid");
  exactKeys(
    multiDocument,
    ["documentCount", "completedHandshakes", "maximumConcurrentHandshakes"],
    "r100_invalid",
  );
  if (multiDocument["documentCount"] !== 16) fail("r100_shape_invalid");
  return Object.freeze({
    subscriptionCount: 100 as const,
    simultaneousContinuityLosses: 100 as const,
    documentReconnectHandshakes: nonnegativeInteger(
      candidate["documentReconnectHandshakes"],
      "r100_invalid",
    ),
    recovery: summary(candidate["recovery"], "r100_recovery_invalid", 300),
    maximumRecoverySkewMs: finite(candidate["maximumRecoverySkewMs"], "r100_invalid"),
    recoveredSubscriptionCount: nonnegativeInteger(
      candidate["recoveredSubscriptionCount"],
      "r100_invalid",
      100,
    ),
    currentSubscriptionCount: nonnegativeInteger(
      candidate["currentSubscriptionCount"],
      "r100_invalid",
      100,
    ),
    starvedSubscriptionCount: nonnegativeInteger(
      candidate["starvedSubscriptionCount"],
      "r100_invalid",
      100,
    ),
    maximumConcurrentReauthorizations: nonnegativeInteger(
      candidate["maximumConcurrentReauthorizations"],
      "r100_invalid",
    ),
    retainedBytesPerIsland: finite(candidate["retainedBytesPerIsland"], "r100_invalid"),
    pollingMaximumSameTick: nonnegativeInteger(candidate["pollingMaximumSameTick"], "r100_invalid"),
    multiDocument: Object.freeze({
      documentCount: 16 as const,
      completedHandshakes: nonnegativeInteger(multiDocument["completedHandshakes"], "r100_invalid"),
      maximumConcurrentHandshakes: nonnegativeInteger(
        multiDocument["maximumConcurrentHandshakes"],
        "r100_invalid",
      ),
    }),
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
      "asyncArtifact",
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
  const asyncArtifact = record(candidate["asyncArtifact"], "async_artifact_invalid");
  exactKeys(asyncArtifact, ["file", "sha256", "brotliBytes"], "async_artifact_invalid");
  if (
    asyncArtifact["file"] !== "suprnova-live.async.esm.js" ||
    !SHA256.test(String(asyncArtifact["sha256"]))
  ) {
    fail("async_artifact_invalid");
  }
  const environment = validateEnvironment(candidate["environment"]);
  const classification = classifyBenchmarkEnvironment(environment);
  if (candidate["classification"] !== classification) fail("classification_invalid");
  const methodology = validateMethodology(candidate["methodology"]);
  const workloads = record(candidate["workloads"], "workloads_invalid");
  exactKeys(workloads, ["D100", "M1K", "M5K", "E100", "R100"], "workloads_invalid");
  const D100 = validateD100(workloads["D100"]);
  const M1K = validateMorph(workloads["M1K"], "M1K");
  const M5K = validateMorph(workloads["M5K"], "M5K");
  const E100 = validateE100(workloads["E100"]);
  const R100 = validateR100(workloads["R100"]);
  const expectedSampleCount = methodology.measuredSamples * methodology.independentRuns;
  for (const item of [D100.connect, M1K.morph, M5K.morph, E100.dispatchEffect, R100.recovery]) {
    if (classification === "b1" && item.sampleCount < 30) fail("sample_count_b1");
    if (item.sampleCount !== expectedSampleCount) fail("sample_count_methodology");
  }
  if (classification === "b1" && methodology.idleDurationMs !== 30_000) {
    fail("idle_duration_b1");
  }

  const independent = record(candidate["independentP95Ms"], "independent_runs_invalid");
  exactKeys(
    independent,
    ["d100Connect", "m1kMorph", "m5kMorph", "e100DispatchEffect", "r100Recovery"],
    "independent_runs_invalid",
  );
  const d100Connect = numberArray(independent["d100Connect"], "independent_runs_invalid", 3);
  const m1kMorph = numberArray(independent["m1kMorph"], "independent_runs_invalid", 3);
  const m5kMorph = numberArray(independent["m5kMorph"], "independent_runs_invalid", 3);
  const e100DispatchEffect = numberArray(
    independent["e100DispatchEffect"],
    "independent_runs_invalid",
    3,
  );
  const r100Recovery = numberArray(independent["r100Recovery"], "independent_runs_invalid", 3);
  if (
    d100Connect.length !== methodology.independentRuns ||
    m1kMorph.length !== methodology.independentRuns ||
    m5kMorph.length !== methodology.independentRuns ||
    e100DispatchEffect.length !== methodology.independentRuns ||
    r100Recovery.length !== methodology.independentRuns
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
    asyncArtifact: Object.freeze({
      file: "suprnova-live.async.esm.js" as const,
      sha256: asyncArtifact["sha256"] as string,
      brotliBytes: positiveInteger(
        asyncArtifact["brotliBytes"],
        "async_artifact_invalid",
        4_194_304,
      ),
    }),
    environment,
    methodology,
    workloads: Object.freeze({ D100, M1K, M5K, E100, R100 }),
    independentP95Ms: Object.freeze({
      d100Connect,
      m1kMorph,
      m5kMorph,
      e100DispatchEffect,
      r100Recovery,
    }),
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
      [
        result.workloads.E100.dispatchEffect.p95Ms,
        BROWSER_BUDGET_LIMITS.e100DispatchEffectP95Ms,
        "e100_dispatch_effect_exceeded",
      ],
      [
        result.workloads.E100.retainedBytesPerSubscription,
        BROWSER_BUDGET_LIMITS.e100RetainedBytesPerSubscription,
        "e100_retained_memory_exceeded",
      ],
      [
        result.workloads.E100.queuedEventPeak,
        BROWSER_BUDGET_LIMITS.e100QueuedEventsPerDocument,
        "e100_queued_events_exceeded",
      ],
      [
        result.workloads.E100.queuedBytePeak,
        BROWSER_BUDGET_LIMITS.e100QueuedBytesPerDocument,
        "e100_queued_bytes_exceeded",
      ],
      [
        result.workloads.E100.maximumQueuedRefreshesPerIsland,
        BROWSER_BUDGET_LIMITS.e100RefreshQueuedPerIsland,
        "e100_refresh_queue_exceeded",
      ],
      [
        result.workloads.E100.maximumInFlightRefreshesPerIsland,
        BROWSER_BUDGET_LIMITS.e100RefreshInFlightPerIsland,
        "e100_refresh_in_flight_exceeded",
      ],
      [
        result.workloads.R100.retainedBytesPerIsland,
        BROWSER_BUDGET_LIMITS.r100RetainedBytesPerIsland,
        "r100_retained_memory_exceeded",
      ],
      [
        result.workloads.R100.multiDocument.maximumConcurrentHandshakes,
        BROWSER_BUDGET_LIMITS.r100OriginConcurrentHandshakes,
        "r100_origin_handshakes_exceeded",
      ],
    ] as const;
    for (const [actual, limit, code] of caps) {
      if (actual > limit) {
        codes.push(code);
        status = "failed";
      }
    }
    const exactEvidence = [
      [result.workloads.E100.physicalConnectionCount, 1, "e100_physical_connection_count"],
      [result.workloads.E100.handshakeCount, 1, "e100_handshake_count"],
      [result.workloads.E100.currentSubscriptionCount, 100, "e100_subscription_state"],
      [result.workloads.R100.documentReconnectHandshakes, 1, "r100_document_reconnect_handshakes"],
      [result.workloads.R100.recoveredSubscriptionCount, 100, "r100_recovery_incomplete"],
      [result.workloads.R100.currentSubscriptionCount, 100, "r100_subscription_state"],
      [result.workloads.R100.starvedSubscriptionCount, 0, "r100_recovery_starvation"],
      [
        result.workloads.R100.multiDocument.completedHandshakes,
        16,
        "r100_multi_document_incomplete",
      ],
      [
        result.workloads.R100.multiDocument.maximumConcurrentHandshakes,
        8,
        "r100_multi_document_fanout",
      ],
    ] as const;
    for (const [actual, expected, code] of exactEvidence) {
      if (actual !== expected) {
        codes.push(code);
        status = "failed";
      }
    }
    if (result.workloads.R100.pollingMaximumSameTick >= 100) {
      codes.push("r100_polling_synchronized_burst");
      status = "failed";
    }
    if (result.workloads.R100.maximumConcurrentReauthorizations > 8) {
      codes.push("r100_reauthorization_concurrency_exceeded");
      status = "failed";
    }
  }
  if (baseline !== undefined && sameEnvironment(result.environment, baseline.environment)) {
    const comparisons = [
      ["d100Connect", baseline.workloads.D100.connect.p95Ms, result.independentP95Ms.d100Connect],
      ["m1kMorph", baseline.workloads.M1K.morph.p95Ms, result.independentP95Ms.m1kMorph],
      ["m5kMorph", baseline.workloads.M5K.morph.p95Ms, result.independentP95Ms.m5kMorph],
      [
        "e100DispatchEffect",
        baseline.workloads.E100.dispatchEffect.p95Ms,
        result.independentP95Ms.e100DispatchEffect,
      ],
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
