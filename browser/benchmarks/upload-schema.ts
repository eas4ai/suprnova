import { estimateUploadManagerOwnedBytes } from "./upload-accounting.js";

export const U4_16 = Object.freeze({
  activeTransfers: 4,
  chunkBytes: 256 * 1024,
  fileBytes: 16 * 1024 * 1024,
  files: 4,
});

export interface UploadBudgetArtifact {
  readonly brotliBytes: number;
  readonly file: "suprnova-live.uploads.esm.js";
  readonly role: "uploads-esm";
  readonly sha256: string;
}

export interface UploadBudgetWorkload {
  readonly activeTransfers: 4;
  readonly chunkBytes: 262144;
  readonly fileBytes: 16777216;
  readonly files: 4;
}

export type UploadBudgetClassification = "qualified" | "unqualified";

export interface UploadBudgetMethodology {
  readonly measuredSamples: number;
  readonly warmupIterations: number;
}

export interface UploadBudgetBrowserMethodology extends UploadBudgetMethodology {
  readonly independentRuns: number;
}

export interface UploadBudgetBrowserEnvironment {
  readonly architecture: string;
  readonly browser: "chromium";
  readonly browserRevision: string;
  readonly classification: UploadBudgetClassification;
  readonly cpuModel: string;
  readonly cpuThrottleRate: 4;
  readonly dedicatedVcpusAttested: boolean;
  readonly extensions: false;
  readonly kernel: string;
  readonly memoryBytes: number;
  readonly operatingSystem: string;
  readonly playwrightVersion: string;
  readonly profile: "B1";
  readonly qualificationRequirementsMet: boolean;
  readonly selectedCpuCount: number;
  readonly viewport: Readonly<{ height: 720; width: 1280 }>;
  readonly warmHttpCache: boolean;
}

export interface UploadBudgetServerEnvironment {
  readonly architecture: string;
  readonly classification: UploadBudgetClassification;
  readonly cpuGovernor: string;
  readonly cpuModel: string;
  readonly database: string;
  readonly dedicatedVcpusAttested: boolean;
  readonly kernel: string;
  readonly loopbackProviders: boolean;
  readonly memoryBytes: number;
  readonly operatingSystem: string;
  readonly profile: "S1";
  readonly qualificationRequirementsMet: boolean;
  readonly rustc: string;
  readonly selectedCpuCount: number;
  readonly warmFilesystemCache: boolean;
}

export interface UploadBudgetManagerOwnedCategories {
  readonly activeLeases: number;
  readonly bindings: number;
  readonly cleanupObligations: number;
  readonly entries: number;
  readonly generationFields: number;
  readonly observers: number;
  readonly ownedResources: number;
  readonly pendingChunkBuffers: number;
  readonly pendingChunkBytes: number;
  readonly queuedBytes: number;
  readonly queuedItems: number;
  readonly retainedStringCodeUnits: number;
  readonly waitingPermits: number;
}

export interface UploadBudgetServerManagerOwnedCategories {
  readonly activePermits: number;
  readonly chunkQueueEntries: number;
  readonly permitSlots: number;
  readonly queueControlRecords: number;
  readonly retainedHandleBytes: number;
  readonly transferQueueEntries: number;
}

export interface UploadBudgetBrowserMeasurements {
  readonly activeTransfers: number;
  readonly liveChunkBuffers: number;
  readonly managerChunkBuffers: number;
  readonly managerOwnedBytes: number;
  readonly managerOwnedCategories: UploadBudgetManagerOwnedCategories;
  readonly maxChunksPerTransfer: number;
  readonly maxConcurrentTransfers: number;
  readonly maxQueueDepth: number;
  readonly progressP50Milliseconds: number;
  readonly progressP95Milliseconds: number;
  readonly retainedBytes: number;
  readonly slicedBytes: number;
  readonly slices: number;
  readonly transportChunkBuffers: number;
}

export interface UploadBudgetBrowserRun {
  readonly artifactSha256: string;
  readonly environment: UploadBudgetBrowserEnvironment;
  readonly measurements: UploadBudgetBrowserMeasurements &
    Readonly<{ progressDurationsMilliseconds: readonly number[] }>;
  readonly methodology: UploadBudgetMethodology;
  readonly runIndex: number;
  readonly workload: UploadBudgetWorkload;
}

export interface UploadBudgetEvidence {
  readonly schemaVersion: 1;
  readonly workload: "U4/16";
  readonly artifact: UploadBudgetArtifact;
  readonly browser: Readonly<{
    bounds: Readonly<{
      maxChunksPerActiveTransfer: 2;
      maxManagerOwnedBytes: 262144;
      maxProgressP95Milliseconds: 16;
    }>;
    environment: UploadBudgetBrowserEnvironment;
    measurements: UploadBudgetBrowserMeasurements;
    methodology: UploadBudgetBrowserMethodology;
    runs: readonly UploadBudgetBrowserRun[];
    workload: UploadBudgetWorkload;
  }>;
  readonly recordedAt: string;
  readonly server: Readonly<{
    bounds: Readonly<{
      maxChunksPerActiveTransfer: 2;
      maxControlP95Microseconds: 2000;
      maxManagerOwnedBytes: 524288;
    }>;
    environment: UploadBudgetServerEnvironment;
    measurements: Readonly<{
      excludedCalls: Readonly<{
        applicationValidation: 0;
        bodyIo: 0;
        provider: 0;
        scanner: 0;
      }>;
      liveChunkBuffers: number;
      managerOwnedBytes: number;
      managerOwnedCategories: UploadBudgetServerManagerOwnedCategories;
      maxChunksPerTransfer: number;
      maxConcurrentTransfers: number;
      maxQueueDepth: number;
      p50Microseconds: number;
      p95Microseconds: number;
      retainedBytes: number;
    }>;
    methodology: UploadBudgetMethodology;
    workload: UploadBudgetWorkload;
  }>;
}

export interface UploadBudgetEvaluation {
  readonly classification: UploadBudgetClassification;
  readonly issues: readonly string[];
}

export interface UploadBudgetBaseline {
  readonly exploratoryReference: UploadBudgetEvidence;
  readonly qualifiedBaseline: UploadBudgetEvidence | null;
  readonly schemaVersion: 1;
  readonly workload: "U4/16";
}

type EvidenceKey =
  | "activeTransfers"
  | "applicationValidation"
  | "architecture"
  | "artifact"
  | "bodyIo"
  | "bounds"
  | "browser"
  | "browserRevision"
  | "brotliBytes"
  | "chunkBytes"
  | "classification"
  | "cpuGovernor"
  | "cpuModel"
  | "cpuThrottleRate"
  | "database"
  | "dedicatedVcpusAttested"
  | "environment"
  | "excludedCalls"
  | "exploratoryReference"
  | "extensions"
  | "file"
  | "fileBytes"
  | "files"
  | "height"
  | "independentRuns"
  | "kernel"
  | "liveChunkBuffers"
  | "managerChunkBuffers"
  | "loopbackProviders"
  | "managerOwnedBytes"
  | "managerOwnedCategories"
  | "maxChunksPerActiveTransfer"
  | "maxChunksPerTransfer"
  | "maxConcurrentTransfers"
  | "maxControlP95Microseconds"
  | "maxManagerOwnedBytes"
  | "maxProgressP95Milliseconds"
  | "maxQueueDepth"
  | "measuredSamples"
  | "measurements"
  | "memoryBytes"
  | "methodology"
  | "operatingSystem"
  | "p50Microseconds"
  | "p95Microseconds"
  | "playwrightVersion"
  | "profile"
  | "progressP50Milliseconds"
  | "progressP95Milliseconds"
  | "provider"
  | "qualificationRequirementsMet"
  | "qualifiedBaseline"
  | "recordedAt"
  | "retainedBytes"
  | "role"
  | "rustc"
  | "scanner"
  | "schemaVersion"
  | "selectedCpuCount"
  | "server"
  | "sha256"
  | "slicedBytes"
  | "slices"
  | "transportChunkBuffers"
  | "viewport"
  | "warmFilesystemCache"
  | "warmHttpCache"
  | "warmupIterations"
  | "width"
  | "workload";

type EvidenceRecord = Record<string, unknown> & Partial<Record<EvidenceKey, unknown>>;

function fail(): never {
  throw new Error("upload_budget_evidence_invalid");
}

function record(value: unknown): EvidenceRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail();
  return value as EvidenceRecord;
}

function exact(value: Record<string, unknown>, keys: readonly string[]): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    fail();
  }
}

function string(value: unknown): string {
  if (typeof value !== "string" || value.length < 1) fail();
  return value;
}

function number(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) fail();
  return value;
}

function integer(value: unknown, minimum = 0): number {
  const candidate = number(value);
  if (!Number.isSafeInteger(candidate) || candidate < minimum) fail();
  return candidate;
}

function boolean(value: unknown): boolean {
  if (typeof value !== "boolean") fail();
  return value;
}

function literal<T extends string | number | boolean>(value: unknown, expected: T): T {
  if (value !== expected) fail();
  return expected;
}

function workload(value: unknown): UploadBudgetWorkload {
  const candidate = record(value);
  exact(candidate, ["activeTransfers", "chunkBytes", "fileBytes", "files"]);
  literal(candidate.activeTransfers, U4_16.activeTransfers);
  literal(candidate.chunkBytes, U4_16.chunkBytes);
  literal(candidate.fileBytes, U4_16.fileBytes);
  literal(candidate.files, U4_16.files);
  return candidate as unknown as UploadBudgetWorkload;
}

function methodology(value: unknown, browser: boolean): UploadBudgetMethodology {
  const candidate = record(value);
  exact(
    candidate,
    browser
      ? ["independentRuns", "measuredSamples", "warmupIterations"]
      : ["measuredSamples", "warmupIterations"],
  );
  integer(candidate.measuredSamples, 30);
  integer(candidate.warmupIterations, 1);
  if (browser) integer(candidate.independentRuns, 1);
  return candidate as unknown as UploadBudgetMethodology;
}

function classification(value: unknown, requirementsMet: boolean): UploadBudgetClassification {
  if (value !== "qualified" && value !== "unqualified") fail();
  if ((value === "qualified") !== requirementsMet) fail();
  return value;
}

function browserEnvironment(value: unknown): UploadBudgetBrowserEnvironment {
  const candidate = record(value);
  exact(candidate, [
    "architecture",
    "browser",
    "browserRevision",
    "classification",
    "cpuModel",
    "cpuThrottleRate",
    "dedicatedVcpusAttested",
    "extensions",
    "kernel",
    "memoryBytes",
    "operatingSystem",
    "playwrightVersion",
    "profile",
    "qualificationRequirementsMet",
    "selectedCpuCount",
    "viewport",
    "warmHttpCache",
  ]);
  const requirementsMet = boolean(candidate.qualificationRequirementsMet);
  classification(candidate.classification, requirementsMet);
  literal(candidate.profile, "B1");
  literal(candidate.browser, "chromium");
  literal(candidate.cpuThrottleRate, 4);
  literal(candidate.extensions, false);
  const viewport = record(candidate.viewport);
  exact(viewport, ["height", "width"]);
  literal(viewport.height, 720);
  literal(viewport.width, 1280);
  const operatingSystem = string(candidate.operatingSystem);
  const architecture = string(candidate.architecture);
  const selectedCpuCount = integer(candidate.selectedCpuCount, 1);
  const memoryBytes = integer(candidate.memoryBytes, 1);
  const dedicated = boolean(candidate.dedicatedVcpusAttested);
  const warmCache = boolean(candidate.warmHttpCache);
  string(candidate.browserRevision);
  string(candidate.cpuModel);
  string(candidate.kernel);
  string(candidate.playwrightVersion);
  const independentlyMet =
    operatingSystem === "linux" &&
    architecture === "x86_64" &&
    selectedCpuCount === 8 &&
    memoryBytes >= 16 * 1024 * 1024 * 1024 &&
    dedicated &&
    warmCache;
  if (requirementsMet !== independentlyMet) fail();
  return candidate as unknown as UploadBudgetBrowserEnvironment;
}

function serverEnvironment(value: unknown): UploadBudgetServerEnvironment {
  const candidate = record(value);
  exact(candidate, [
    "architecture",
    "classification",
    "cpuGovernor",
    "cpuModel",
    "database",
    "dedicatedVcpusAttested",
    "kernel",
    "loopbackProviders",
    "memoryBytes",
    "operatingSystem",
    "profile",
    "qualificationRequirementsMet",
    "rustc",
    "selectedCpuCount",
    "warmFilesystemCache",
  ]);
  const requirementsMet = boolean(candidate.qualificationRequirementsMet);
  classification(candidate.classification, requirementsMet);
  literal(candidate.profile, "S1");
  const operatingSystem = string(candidate.operatingSystem);
  const architecture = string(candidate.architecture);
  const selectedCpuCount = integer(candidate.selectedCpuCount, 1);
  const memoryBytes = integer(candidate.memoryBytes, 1);
  const governor = string(candidate.cpuGovernor);
  const dedicated = boolean(candidate.dedicatedVcpusAttested);
  const warmCache = boolean(candidate.warmFilesystemCache);
  const loopback = boolean(candidate.loopbackProviders);
  string(candidate.cpuModel);
  string(candidate.database);
  string(candidate.kernel);
  string(candidate.rustc);
  const independentlyMet =
    operatingSystem === "linux" &&
    architecture === "x86_64" &&
    selectedCpuCount === 8 &&
    memoryBytes >= 16 * 1024 * 1024 * 1024 &&
    governor === "performance" &&
    dedicated &&
    warmCache &&
    loopback;
  if (requirementsMet !== independentlyMet) fail();
  return candidate as unknown as UploadBudgetServerEnvironment;
}

function managerCategories(value: unknown): UploadBudgetManagerOwnedCategories {
  const candidate = record(value);
  exact(candidate, [
    "activeLeases",
    "bindings",
    "cleanupObligations",
    "entries",
    "generationFields",
    "observers",
    "ownedResources",
    "pendingChunkBuffers",
    "pendingChunkBytes",
    "queuedBytes",
    "queuedItems",
    "retainedStringCodeUnits",
    "waitingPermits",
  ]);
  for (const value of Object.values(candidate)) integer(value);
  return candidate as unknown as UploadBudgetManagerOwnedCategories;
}

function serverManagerCategories(value: unknown): UploadBudgetServerManagerOwnedCategories {
  const candidate = record(value);
  exact(candidate, [
    "activePermits",
    "chunkQueueEntries",
    "permitSlots",
    "queueControlRecords",
    "retainedHandleBytes",
    "transferQueueEntries",
  ]);
  for (const value of Object.values(candidate)) integer(value);
  return candidate as unknown as UploadBudgetServerManagerOwnedCategories;
}

function estimateServerManagerOwnedBytes(
  categories: UploadBudgetServerManagerOwnedCategories,
): number {
  return (
    categories.queueControlRecords * 512 +
    categories.transferQueueEntries * 256 +
    categories.chunkQueueEntries * 128 +
    categories.activePermits * 128 +
    categories.permitSlots * 128 +
    categories.retainedHandleBytes
  );
}

function browserMeasurements(
  value: unknown,
  samplesRequired: boolean,
): UploadBudgetBrowserMeasurements {
  const candidate = record(value);
  exact(candidate, [
    "activeTransfers",
    "liveChunkBuffers",
    "managerChunkBuffers",
    "managerOwnedBytes",
    "managerOwnedCategories",
    "maxChunksPerTransfer",
    "maxConcurrentTransfers",
    "maxQueueDepth",
    "progressP50Milliseconds",
    "progressP95Milliseconds",
    ...(samplesRequired ? ["progressDurationsMilliseconds"] : []),
    "retainedBytes",
    "slicedBytes",
    "slices",
    "transportChunkBuffers",
  ]);
  literal(candidate.activeTransfers, U4_16.activeTransfers);
  const liveChunkBuffers = integer(candidate.liveChunkBuffers, 1);
  const managerChunkBuffers = integer(candidate.managerChunkBuffers, 1);
  const transportChunkBuffers = integer(candidate.transportChunkBuffers, 1);
  if (
    liveChunkBuffers !== managerChunkBuffers + transportChunkBuffers ||
    liveChunkBuffers > U4_16.activeTransfers * 2
  ) {
    fail();
  }
  const categories = managerCategories(candidate.managerOwnedCategories);
  literal(candidate.managerOwnedBytes, estimateUploadManagerOwnedBytes(categories));
  literal(candidate.maxChunksPerTransfer, 2);
  literal(candidate.maxConcurrentTransfers, U4_16.activeTransfers);
  literal(candidate.maxQueueDepth, U4_16.files);
  number(candidate.progressP50Milliseconds);
  number(candidate.progressP95Milliseconds);
  integer(candidate.retainedBytes);
  literal(candidate.slicedBytes, U4_16.files * U4_16.fileBytes);
  literal(candidate.slices, U4_16.files * (U4_16.fileBytes / U4_16.chunkBytes));
  if (number(candidate.progressP50Milliseconds) > number(candidate.progressP95Milliseconds)) fail();
  const minimumRetained =
    liveChunkBuffers * U4_16.chunkBytes + integer(candidate.managerOwnedBytes);
  if (integer(candidate.retainedBytes) < minimumRetained) fail();
  if (samplesRequired) {
    if (!Array.isArray(candidate["progressDurationsMilliseconds"])) fail();
    const samples = candidate["progressDurationsMilliseconds"].map(number);
    const summary = summarizeUploadSamples(samples);
    if (
      summary.p50 !== candidate.progressP50Milliseconds ||
      summary.p95 !== candidate.progressP95Milliseconds
    ) {
      fail();
    }
  }
  return candidate as unknown as UploadBudgetBrowserMeasurements;
}

function equalBrowserEnvironment(
  left: UploadBudgetBrowserEnvironment,
  right: UploadBudgetBrowserEnvironment,
): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function validateBrowser(value: unknown, artifactSha256: string): UploadBudgetEvidence["browser"] {
  const candidate = record(value);
  exact(candidate, ["bounds", "environment", "measurements", "methodology", "runs", "workload"]);
  workload(candidate.workload);
  const aggregateMethodology = methodology(
    candidate.methodology,
    true,
  ) as UploadBudgetBrowserMethodology;
  const aggregateEnvironment = browserEnvironment(candidate.environment);

  const bounds = record(candidate.bounds);
  exact(bounds, [
    "maxChunksPerActiveTransfer",
    "maxManagerOwnedBytes",
    "maxProgressP95Milliseconds",
  ]);
  literal(bounds.maxChunksPerActiveTransfer, 2);
  literal(bounds.maxManagerOwnedBytes, 256 * 1024);
  literal(bounds.maxProgressP95Milliseconds, 16);

  const measurements = browserMeasurements(candidate.measurements, false);
  if (!Array.isArray(candidate["runs"]) || candidate["runs"].length < 1) fail();
  if (candidate["runs"].length !== aggregateMethodology.independentRuns) fail();
  if (aggregateEnvironment.classification === "qualified" && candidate["runs"].length !== 3) fail();
  const runs = candidate["runs"].map((value, index): UploadBudgetBrowserRun => {
    const run = record(value);
    exact(run, [
      "artifactSha256",
      "environment",
      "measurements",
      "methodology",
      "runIndex",
      "workload",
    ]);
    literal(run["runIndex"], index + 1);
    literal(run["artifactSha256"], artifactSha256);
    workload(run.workload);
    const runEnvironment = browserEnvironment(run.environment);
    if (!equalBrowserEnvironment(runEnvironment, aggregateEnvironment)) fail();
    const runMethodology = methodology(run.methodology, false);
    literal(runMethodology.warmupIterations, 5);
    const runMeasurements = browserMeasurements(
      run.measurements,
      true,
    ) as UploadBudgetBrowserRun["measurements"];
    if (runMeasurements.progressDurationsMilliseconds.length !== runMethodology.measuredSamples)
      fail();
    return run as unknown as UploadBudgetBrowserRun;
  });
  const samples = runs.flatMap((run) => run.measurements.progressDurationsMilliseconds);
  literal(aggregateMethodology.measuredSamples, samples.length);
  literal(aggregateMethodology.warmupIterations, 5);
  const aggregate = summarizeUploadSamples(samples);
  if (
    aggregate.p50 !== measurements.progressP50Milliseconds ||
    aggregate.p95 !== measurements.progressP95Milliseconds
  ) {
    fail();
  }
  const maximum = (key: keyof UploadBudgetBrowserMeasurements): number =>
    Math.max(...runs.map((run) => run.measurements[key] as number));
  for (const key of [
    "liveChunkBuffers",
    "managerChunkBuffers",
    "managerOwnedBytes",
    "maxChunksPerTransfer",
    "maxConcurrentTransfers",
    "maxQueueDepth",
    "retainedBytes",
    "transportChunkBuffers",
  ] as const) {
    if (measurements[key] !== maximum(key)) fail();
  }
  return candidate as unknown as UploadBudgetEvidence["browser"];
}

function validateServer(value: unknown): UploadBudgetEvidence["server"] {
  const candidate = record(value);
  exact(candidate, ["bounds", "environment", "measurements", "methodology", "workload"]);
  workload(candidate.workload);
  methodology(candidate.methodology, false);
  serverEnvironment(candidate.environment);

  const bounds = record(candidate.bounds);
  exact(bounds, [
    "maxChunksPerActiveTransfer",
    "maxControlP95Microseconds",
    "maxManagerOwnedBytes",
  ]);
  literal(bounds.maxChunksPerActiveTransfer, 2);
  literal(bounds.maxControlP95Microseconds, 2_000);
  literal(bounds.maxManagerOwnedBytes, 512 * 1024);

  const measurements = record(candidate.measurements);
  exact(measurements, [
    "excludedCalls",
    "liveChunkBuffers",
    "managerOwnedBytes",
    "managerOwnedCategories",
    "maxChunksPerTransfer",
    "maxConcurrentTransfers",
    "maxQueueDepth",
    "p50Microseconds",
    "p95Microseconds",
    "retainedBytes",
  ]);
  const excluded = record(measurements.excludedCalls);
  exact(excluded, ["applicationValidation", "bodyIo", "provider", "scanner"]);
  literal(excluded.applicationValidation, 0);
  literal(excluded.bodyIo, 0);
  literal(excluded.provider, 0);
  literal(excluded.scanner, 0);
  literal(measurements.liveChunkBuffers, U4_16.activeTransfers * 2);
  const categories = serverManagerCategories(measurements.managerOwnedCategories);
  literal(categories.activePermits, U4_16.activeTransfers);
  literal(categories.chunkQueueEntries, U4_16.activeTransfers * 2);
  literal(categories.permitSlots, U4_16.activeTransfers);
  literal(categories.queueControlRecords, 2);
  literal(categories.transferQueueEntries, U4_16.activeTransfers);
  literal(measurements.managerOwnedBytes, estimateServerManagerOwnedBytes(categories));
  literal(measurements.maxChunksPerTransfer, 2);
  literal(measurements.maxConcurrentTransfers, U4_16.activeTransfers);
  literal(measurements.maxQueueDepth, U4_16.activeTransfers);
  number(measurements.p50Microseconds);
  number(measurements.p95Microseconds);
  const expectedRetained =
    U4_16.activeTransfers * 2 * U4_16.chunkBytes +
    integer(measurements.managerOwnedBytes) +
    categories.retainedHandleBytes;
  literal(measurements.retainedBytes, expectedRetained);
  if (number(measurements.p50Microseconds) > number(measurements.p95Microseconds)) fail();
  return candidate as unknown as UploadBudgetEvidence["server"];
}

export function validateUploadBudgetEvidence(value: unknown): UploadBudgetEvidence {
  const candidate = record(value);
  exact(candidate, ["artifact", "browser", "recordedAt", "schemaVersion", "server", "workload"]);
  literal(candidate.schemaVersion, 1);
  literal(candidate.workload, "U4/16");
  if (Number.isNaN(Date.parse(string(candidate.recordedAt)))) fail();

  const artifact = record(candidate.artifact);
  exact(artifact, ["brotliBytes", "file", "role", "sha256"]);
  integer(artifact.brotliBytes, 1);
  literal(artifact.file, "suprnova-live.uploads.esm.js");
  literal(artifact.role, "uploads-esm");
  if (!/^[0-9a-f]{64}$/u.test(string(artifact.sha256))) fail();

  const browser = validateBrowser(candidate.browser, string(artifact.sha256));
  const server = validateServer(candidate.server);
  if (
    browser.environment.classification === "qualified" &&
    server.environment.classification === "qualified" &&
    (browser.environment.architecture !== server.environment.architecture ||
      browser.environment.cpuModel !== server.environment.cpuModel ||
      browser.environment.kernel !== server.environment.kernel ||
      browser.environment.memoryBytes !== server.environment.memoryBytes ||
      browser.environment.operatingSystem !== server.environment.operatingSystem ||
      browser.environment.selectedCpuCount !== server.environment.selectedCpuCount)
  ) {
    fail();
  }
  return candidate as unknown as UploadBudgetEvidence;
}

export function validateUploadBudgetBaseline(value: unknown): UploadBudgetBaseline {
  try {
    const candidate = record(value);
    exact(candidate, ["exploratoryReference", "qualifiedBaseline", "schemaVersion", "workload"]);
    literal(candidate.schemaVersion, 1);
    literal(candidate.workload, "U4/16");
    const exploratoryReference = validateUploadBudgetEvidence(candidate.exploratoryReference);
    if (
      exploratoryReference.browser.environment.classification !== "unqualified" ||
      exploratoryReference.server.environment.classification !== "unqualified"
    ) {
      throw new Error("upload_budget_baseline_invalid");
    }
    const qualifiedBaseline =
      candidate.qualifiedBaseline === null
        ? null
        : validateUploadBudgetEvidence(candidate.qualifiedBaseline);
    if (
      qualifiedBaseline !== null &&
      (qualifiedBaseline.browser.environment.classification !== "qualified" ||
        qualifiedBaseline.server.environment.classification !== "qualified" ||
        qualifiedBaseline.browser.methodology.independentRuns < 3)
    ) {
      throw new Error("upload_budget_baseline_invalid");
    }
    return Object.freeze({
      exploratoryReference,
      qualifiedBaseline,
      schemaVersion: 1,
      workload: "U4/16",
    });
  } catch {
    throw new Error("upload_budget_baseline_invalid");
  }
}

export function summarizeUploadSamples(
  samples: readonly number[],
): Readonly<{ p50: number; p95: number }> {
  if (samples.length < 1 || samples.some((sample) => !Number.isFinite(sample) || sample < 0)) {
    fail();
  }
  const sorted = [...samples].sort((left, right) => left - right);
  const percentile = (fraction: number): number => {
    const index = Math.max(0, Math.ceil(sorted.length * fraction) - 1);
    const value = sorted[index];
    if (value === undefined) fail();
    return value;
  };
  return Object.freeze({ p50: percentile(0.5), p95: percentile(0.95) });
}

export function regressionAtLeast15Percent(candidate: number, baseline: number): boolean {
  if (!Number.isFinite(candidate) || candidate < 0 || !Number.isFinite(baseline) || baseline < 0) {
    fail();
  }
  if (baseline === 0) return candidate > 0;
  const candidateUnits = BigInt(Math.round(candidate * 1_000_000_000));
  const baselineUnits = BigInt(Math.round(baseline * 1_000_000_000));
  return candidateUnits * 10_000n >= baselineUnits * 11_500n;
}

function sameEnvironment(candidate: UploadBudgetEvidence, baseline: UploadBudgetEvidence): boolean {
  const candidateBrowser = candidate.browser.environment;
  const baselineBrowser = baseline.browser.environment;
  const candidateServer = candidate.server.environment;
  const baselineServer = baseline.server.environment;
  return (
    candidateBrowser.architecture === baselineBrowser.architecture &&
    candidateBrowser.browserRevision === baselineBrowser.browserRevision &&
    candidateBrowser.cpuModel === baselineBrowser.cpuModel &&
    candidateBrowser.kernel === baselineBrowser.kernel &&
    candidateBrowser.memoryBytes === baselineBrowser.memoryBytes &&
    candidateBrowser.operatingSystem === baselineBrowser.operatingSystem &&
    candidateBrowser.playwrightVersion === baselineBrowser.playwrightVersion &&
    candidateBrowser.selectedCpuCount === baselineBrowser.selectedCpuCount &&
    candidateServer.architecture === baselineServer.architecture &&
    candidateServer.cpuGovernor === baselineServer.cpuGovernor &&
    candidateServer.cpuModel === baselineServer.cpuModel &&
    candidateServer.kernel === baselineServer.kernel &&
    candidateServer.memoryBytes === baselineServer.memoryBytes &&
    candidateServer.operatingSystem === baselineServer.operatingSystem &&
    candidateServer.rustc === baselineServer.rustc &&
    candidateServer.selectedCpuCount === baselineServer.selectedCpuCount
  );
}

export function evaluateUploadBudget(
  candidate: UploadBudgetEvidence,
  baseline: UploadBudgetEvidence | null,
  options: Readonly<{ artifactSha256?: string; release?: boolean }> = {},
): UploadBudgetEvaluation {
  const issues: string[] = [];
  if (
    options.artifactSha256 !== undefined &&
    candidate.artifact.sha256 !== options.artifactSha256
  ) {
    issues.push("upload_budget:artifact_mismatch");
  }
  const browser = candidate.browser.measurements;
  if (browser.liveChunkBuffers > U4_16.activeTransfers * 2) {
    issues.push("upload_budget:browser:live_chunk_buffers_hard_cap");
  }
  if (browser.maxChunksPerTransfer > 2) {
    issues.push("upload_budget:browser:chunks_per_transfer_hard_cap");
  }
  if (browser.managerOwnedBytes > 256 * 1024) {
    issues.push("upload_budget:browser:manager_bytes_hard_cap");
  }
  if (browser.progressP95Milliseconds > 16) {
    issues.push("upload_budget:browser:progress_p95_hard_cap");
  }
  const server = candidate.server.measurements;
  if (server.liveChunkBuffers > U4_16.activeTransfers * 2) {
    issues.push("upload_budget:server:live_chunk_buffers_hard_cap");
  }
  if (server.maxChunksPerTransfer > 2) {
    issues.push("upload_budget:server:chunks_per_transfer_hard_cap");
  }
  if (server.managerOwnedBytes > 512 * 1024) {
    issues.push("upload_budget:server:manager_bytes_hard_cap");
  }
  if (server.p95Microseconds > 2_000) {
    issues.push("upload_budget:server:control_p95_hard_cap");
  }
  if (baseline !== null) {
    if (!sameEnvironment(candidate, baseline)) {
      issues.push("upload_budget:baseline_environment_mismatch");
    } else {
      if (
        regressionAtLeast15Percent(
          browser.progressP95Milliseconds,
          baseline.browser.measurements.progressP95Milliseconds,
        )
      ) {
        issues.push("upload_budget:browser:progress_p95_regression");
      }
      if (candidate.browser.runs.length !== baseline.browser.runs.length) {
        issues.push("upload_budget:browser:run_count_mismatch");
      } else {
        for (let index = 0; index < candidate.browser.runs.length; index += 1) {
          const candidateRun = candidate.browser.runs[index];
          const baselineRun = baseline.browser.runs[index];
          if (candidateRun === undefined || baselineRun === undefined) fail();
          if (
            candidateRun.artifactSha256 !== candidate.artifact.sha256 ||
            baselineRun.artifactSha256 !== baseline.artifact.sha256 ||
            !equalBrowserEnvironment(candidateRun.environment, baselineRun.environment) ||
            candidateRun.methodology.measuredSamples !==
              candidateRun.measurements.progressDurationsMilliseconds.length ||
            baselineRun.methodology.measuredSamples !==
              baselineRun.measurements.progressDurationsMilliseconds.length
          ) {
            issues.push(`upload_budget:browser:run_${String(index + 1)}:evidence_mismatch`);
            continue;
          }
          if (
            regressionAtLeast15Percent(
              candidateRun.measurements.progressP95Milliseconds,
              baselineRun.measurements.progressP95Milliseconds,
            )
          ) {
            issues.push(`upload_budget:browser:run_${String(index + 1)}:progress_p95_regression`);
          }
        }
      }
      if (
        regressionAtLeast15Percent(
          server.p95Microseconds,
          baseline.server.measurements.p95Microseconds,
        )
      ) {
        issues.push("upload_budget:server:control_p95_regression");
      }
    }
  }
  const qualified =
    candidate.browser.environment.classification === "qualified" &&
    candidate.server.environment.classification === "qualified" &&
    candidate.browser.methodology.independentRuns >= 3;
  if (options.release === true && !qualified) {
    issues.push("upload_budget:release_environment_unqualified");
  }
  if (options.release === true && baseline === null) {
    issues.push("upload_budget:qualified_baseline_missing");
  }
  return Object.freeze({
    classification: qualified ? "qualified" : "unqualified",
    issues: Object.freeze(issues),
  });
}
