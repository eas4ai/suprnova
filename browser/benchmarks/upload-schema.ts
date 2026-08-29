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
    measurements: Readonly<{
      activeTransfers: number;
      liveChunkBuffers: number;
      managerOwnedBytes: number;
      maxChunksPerTransfer: number;
      maxConcurrentTransfers: number;
      maxQueueDepth: number;
      progressP50Milliseconds: number;
      progressP95Milliseconds: number;
      retainedBytes: number;
      slicedBytes: number;
      slices: number;
    }>;
    methodology: UploadBudgetBrowserMethodology;
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
  | "loopbackProviders"
  | "managerOwnedBytes"
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
  | "viewport"
  | "warmFilesystemCache"
  | "warmHttpCache"
  | "warmupIterations"
  | "width"
  | "workload";

type EvidenceRecord = Record<string, unknown> & Record<EvidenceKey, unknown>;

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

function validateBrowser(value: unknown): UploadBudgetEvidence["browser"] {
  const candidate = record(value);
  exact(candidate, ["bounds", "environment", "measurements", "methodology", "workload"]);
  workload(candidate.workload);
  methodology(candidate.methodology, true);
  browserEnvironment(candidate.environment);

  const bounds = record(candidate.bounds);
  exact(bounds, [
    "maxChunksPerActiveTransfer",
    "maxManagerOwnedBytes",
    "maxProgressP95Milliseconds",
  ]);
  literal(bounds.maxChunksPerActiveTransfer, 2);
  literal(bounds.maxManagerOwnedBytes, 256 * 1024);
  literal(bounds.maxProgressP95Milliseconds, 16);

  const measurements = record(candidate.measurements);
  exact(measurements, [
    "activeTransfers",
    "liveChunkBuffers",
    "managerOwnedBytes",
    "maxChunksPerTransfer",
    "maxConcurrentTransfers",
    "maxQueueDepth",
    "progressP50Milliseconds",
    "progressP95Milliseconds",
    "retainedBytes",
    "slicedBytes",
    "slices",
  ]);
  literal(measurements.activeTransfers, 4);
  literal(measurements.liveChunkBuffers, 2);
  literal(measurements.managerOwnedBytes, 256 * 1024);
  literal(measurements.maxChunksPerTransfer, 2);
  integer(measurements.maxConcurrentTransfers);
  literal(measurements.maxQueueDepth, U4_16.files);
  number(measurements.progressP50Milliseconds);
  number(measurements.progressP95Milliseconds);
  integer(measurements.retainedBytes);
  literal(measurements.slicedBytes, U4_16.files * U4_16.fileBytes);
  literal(measurements.slices, U4_16.files * (U4_16.fileBytes / U4_16.chunkBytes));
  literal(measurements.maxConcurrentTransfers, U4_16.activeTransfers);
  if (number(measurements.progressP50Milliseconds) > number(measurements.progressP95Milliseconds)) {
    fail();
  }
  const minimumRetained =
    integer(measurements.liveChunkBuffers) * U4_16.chunkBytes +
    integer(measurements.managerOwnedBytes);
  if (integer(measurements.retainedBytes) < minimumRetained) fail();
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
  literal(measurements.liveChunkBuffers, 0);
  literal(measurements.managerOwnedBytes, 0);
  literal(measurements.maxChunksPerTransfer, 0);
  literal(measurements.maxConcurrentTransfers, 0);
  literal(measurements.maxQueueDepth, 0);
  number(measurements.p50Microseconds);
  number(measurements.p95Microseconds);
  literal(measurements.retainedBytes, 0);
  if (number(measurements.p50Microseconds) > number(measurements.p95Microseconds)) fail();
  const minimumRetained =
    integer(measurements.liveChunkBuffers) * U4_16.chunkBytes +
    integer(measurements.managerOwnedBytes);
  if (integer(measurements.retainedBytes) < minimumRetained) fail();
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

  const browser = validateBrowser(candidate.browser);
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

function regression(candidate: number, baseline: number): boolean {
  return candidate > baseline * 1.15;
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
        regression(
          browser.progressP95Milliseconds,
          baseline.browser.measurements.progressP95Milliseconds,
        )
      ) {
        issues.push("upload_budget:browser:progress_p95_regression");
      }
      if (regression(server.p95Microseconds, baseline.server.measurements.p95Microseconds)) {
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
