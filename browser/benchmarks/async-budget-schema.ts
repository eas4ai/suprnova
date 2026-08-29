export const E100_1K = Object.freeze({
  subscriptions: 100,
  documentTransports: 1,
  presentationEvents: 1_000,
  payloadBytes: 1_024,
  durationMs: 10_000,
  refreshInvalidations: 100,
  maxRetainedBytesPerSubscription: 8 * 1_024,
  maxDocumentEvents: 64,
  maxDocumentBytes: 256 * 1_024,
  maxDispatchP95Milliseconds: 8,
  maxQueuedRefreshesPerIsland: 1,
  maxInFlightRefreshesPerIsland: 1,
});

export const R100 = Object.freeze({
  subscriptions: 100,
  reconnectHandshakes: 1,
  maxConcurrentHandshakesPerOrigin: 8,
  maxRetainedBytesAfterCurrent: 12 * 1_024,
});

export const ASYNC_MULTI_DOCUMENT = Object.freeze({
  documents: 16,
  attemptedHandshakes: 16,
  maxConcurrentHandshakesPerOrigin: 8,
});

export type AsyncBudgetClassification = "qualified" | "unqualified";
export type AsyncBudgetStatus = "passed" | "unqualified" | "failed";

export interface AsyncBudgetEvaluation {
  readonly classification: AsyncBudgetClassification;
  readonly issues: readonly string[];
  readonly observations: readonly string[];
  readonly status: AsyncBudgetStatus;
}

export interface AsyncBudgetEvidence {
  readonly schemaVersion: 1;
  readonly suite: "E100/1K+R100";
  readonly artifact: Readonly<Record<string, unknown>>;
  readonly environment: Readonly<Record<string, unknown>>;
  readonly methodology: Readonly<Record<string, unknown>>;
  readonly e100: Readonly<Record<string, unknown>>;
  readonly r100: Readonly<Record<string, unknown>>;
  readonly multiDocument: Readonly<Record<string, unknown>>;
  readonly mutationProofs: Readonly<Record<string, unknown>>;
  readonly recordedAt: string;
  readonly runs: readonly Readonly<Record<string, unknown>>[];
}

export interface AsyncBudgetBaseline {
  readonly exploratoryReference: AsyncBudgetEvidence;
  readonly qualifiedBaseline: AsyncBudgetEvidence | null;
  readonly schemaVersion: 1;
  readonly suite: "E100/1K+R100";
}

const SHA256 = /^[0-9a-f]{64}$/u;
const SUBSCRIPTION = /^subscription-[0-9]{3}$/u;

function fail(): never {
  throw new Error("async_budget_evidence_invalid");
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail();
  return value as Record<string, unknown>;
}

function exact(candidate: Record<string, unknown>, keys: readonly string[]): void {
  const actual = Object.keys(candidate).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    fail();
  }
}

function literal<T extends string | number | boolean>(value: unknown, expected: T): T {
  if (value !== expected) fail();
  return expected;
}

function text(value: unknown): string {
  if (typeof value !== "string" || value.length === 0) fail();
  return value;
}

function integer(value: unknown, minimum = 0, maximum = Number.MAX_SAFE_INTEGER): number {
  if (!Number.isSafeInteger(value) || Number(value) < minimum || Number(value) > maximum) fail();
  return Number(value);
}

function finite(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) fail();
  return value;
}

function boolean(value: unknown): boolean {
  if (typeof value !== "boolean") fail();
  return value;
}

function sha(value: unknown): string {
  const candidate = text(value);
  if (!SHA256.test(candidate)) fail();
  return candidate;
}

function array(value: unknown, length?: number): readonly unknown[] {
  if (!Array.isArray(value) || (length !== undefined && value.length !== length)) fail();
  return value;
}

function artifact(value: unknown): void {
  const candidate = record(value);
  exact(candidate, [
    "brotliBytes",
    "file",
    "manifestSha256",
    "role",
    "sha256",
    "sourceInputsSha256",
  ]);
  integer(candidate["brotliBytes"], 1);
  literal(candidate["file"], "suprnova-live.async.esm.js");
  literal(candidate["role"], "async-esm");
  sha(candidate["sha256"]);
  sha(candidate["manifestSha256"]);
  sha(candidate["sourceInputsSha256"]);
}

function environment(value: unknown): AsyncBudgetClassification {
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
  literal(candidate["profile"], "B1");
  literal(candidate["browser"], "chromium");
  literal(candidate["cpuThrottleRate"], 4);
  literal(candidate["extensions"], false);
  const qualification = boolean(candidate["qualificationRequirementsMet"]);
  const classification = candidate["classification"];
  if (classification !== "qualified" && classification !== "unqualified") fail();
  if ((classification === "qualified") !== qualification) fail();
  const operatingSystem = text(candidate["operatingSystem"]);
  const architecture = text(candidate["architecture"]);
  const selectedCpuCount = integer(candidate["selectedCpuCount"], 1);
  const memoryBytes = integer(candidate["memoryBytes"], 1);
  const dedicated = boolean(candidate["dedicatedVcpusAttested"]);
  const warmHttpCache = boolean(candidate["warmHttpCache"]);
  text(candidate["browserRevision"]);
  text(candidate["cpuModel"]);
  text(candidate["kernel"]);
  text(candidate["playwrightVersion"]);
  const viewport = record(candidate["viewport"]);
  exact(viewport, ["height", "width"]);
  literal(viewport["height"], 720);
  literal(viewport["width"], 1_280);
  const independentlyQualified =
    operatingSystem === "linux" &&
    architecture === "x86_64" &&
    selectedCpuCount === 8 &&
    memoryBytes >= 16 * 1_024 * 1_024 * 1_024 &&
    dedicated &&
    warmHttpCache;
  if (qualification !== independentlyQualified) fail();
  return classification;
}

function methodology(value: unknown, classification: AsyncBudgetClassification): number {
  const candidate = record(value);
  exact(candidate, [
    "controlledTimeline",
    "independentRuns",
    "measuredSamples",
    "monotonicClock",
    "retainedHeap",
    "regressionReference",
    "warmupIterations",
    "watchdogOutsideSamples",
  ]);
  literal(candidate["controlledTimeline"], true);
  literal(candidate["measuredSamples"], 1_000);
  literal(candidate["monotonicClock"], "performance.now");
  literal(candidate["regressionReference"], "median_run_p95_v1");
  literal(candidate["watchdogOutsideSamples"], true);
  const retainedHeap = record(candidate["retainedHeap"]);
  exact(retainedHeap, [
    "api",
    "beforeState",
    "derivation",
    "exclusions",
    "garbageCollection",
    "harnessTreatment",
    "phaseSamples",
    "product",
    "protocolVersion",
    "unavailable",
  ]);
  literal(retainedHeap["api"], "Chromium CDP Runtime.getHeapUsage");
  literal(retainedHeap["derivation"], "max_after_total_minus_min_before_total");
  literal(retainedHeap["garbageCollection"], "HeapProfiler.collectGarbage");
  literal(retainedHeap["phaseSamples"], 5);
  literal(retainedHeap["unavailable"], "fail_closed");
  text(retainedHeap["beforeState"]);
  text(retainedHeap["harnessTreatment"]);
  text(retainedHeap["product"]);
  text(retainedHeap["protocolVersion"]);
  const exclusions = array(retainedHeap["exclusions"], 3);
  literal(exclusions[0], "native_transport");
  literal(exclusions[1], "DOM");
  literal(exclusions[2], "current_payload");
  integer(candidate["warmupIterations"], 1);
  const independentRuns = integer(candidate["independentRuns"], 1, 3);
  if (classification === "qualified" && independentRuns !== 3) fail();
  return independentRuns;
}

function heapSample(value: unknown): number {
  const candidate = record(value);
  exact(candidate, ["backingStorageSize", "embedderHeapUsedSize", "usedSize"]);
  const total =
    integer(candidate["backingStorageSize"]) +
    integer(candidate["embedderHeapUsedSize"]) +
    integer(candidate["usedSize"]);
  if (!Number.isSafeInteger(total)) fail();
  return total;
}

function retainedHeap(value: unknown): number {
  const candidate = record(value);
  exact(candidate, ["after", "before", "retainedBytes"]);
  const before = array(candidate["before"], 5).map(heapSample);
  const after = array(candidate["after"], 5).map(heapSample);
  const derived = Math.max(...after) - Math.min(...before);
  if (!Number.isSafeInteger(derived) || derived < 0) fail();
  literal(candidate["retainedBytes"], derived);
  return derived;
}

function sampleSummary(value: unknown, expectedSamples: number): void {
  const candidate = record(value);
  exact(candidate, ["durationsMilliseconds", "p50Milliseconds", "p95Milliseconds", "sampleCount"]);
  const durations = array(candidate["durationsMilliseconds"], expectedSamples).map(finite);
  literal(candidate["sampleCount"], expectedSamples);
  const ordered = [...durations].sort((left, right) => left - right);
  const percentile = (quantile: number): number => {
    const value = ordered[Math.max(0, Math.ceil(quantile * ordered.length) - 1)];
    if (value === undefined) fail();
    return value;
  };
  if (
    Math.abs(finite(candidate["p50Milliseconds"]) - percentile(0.5)) > 0.000_001 ||
    Math.abs(finite(candidate["p95Milliseconds"]) - percentile(0.95)) > 0.000_001
  ) {
    fail();
  }
}

function e100(value: unknown, artifactSha256: string): ReadonlyMap<string, number> {
  const candidate = record(value);
  exact(candidate, ["bounds", "measurements", "workload"]);
  const workload = record(candidate["workload"]);
  exact(workload, [
    "documentTransports",
    "durationMs",
    "payloadBytes",
    "presentationEvents",
    "refreshInvalidations",
    "subscriptions",
  ]);
  literal(workload["subscriptions"], E100_1K.subscriptions);
  literal(workload["documentTransports"], E100_1K.documentTransports);
  literal(workload["presentationEvents"], E100_1K.presentationEvents);
  literal(workload["payloadBytes"], E100_1K.payloadBytes);
  literal(workload["durationMs"], E100_1K.durationMs);
  literal(workload["refreshInvalidations"], E100_1K.refreshInvalidations);
  const bounds = record(candidate["bounds"]);
  exact(bounds, [
    "maxDispatchP95Milliseconds",
    "maxDocumentBytes",
    "maxDocumentEvents",
    "maxInFlightRefreshesPerIsland",
    "maxQueuedRefreshesPerIsland",
    "maxRetainedBytesPerSubscription",
  ]);
  for (const [key, expected] of Object.entries({
    maxDispatchP95Milliseconds: E100_1K.maxDispatchP95Milliseconds,
    maxDocumentBytes: E100_1K.maxDocumentBytes,
    maxDocumentEvents: E100_1K.maxDocumentEvents,
    maxInFlightRefreshesPerIsland: E100_1K.maxInFlightRefreshesPerIsland,
    maxQueuedRefreshesPerIsland: E100_1K.maxQueuedRefreshesPerIsland,
    maxRetainedBytesPerSubscription: E100_1K.maxRetainedBytesPerSubscription,
  })) {
    literal(bounds[key], expected);
  }
  const measurements = record(candidate["measurements"]);
  exact(measurements, ["dispatch", "document", "rustOwner", "subscriptions"]);
  sampleSummary(measurements["dispatch"], E100_1K.presentationEvents);
  const document = record(measurements["document"]);
  exact(document, [
    "activeTransportOwners",
    "currentPayloadOwners",
    "fairnessMaximumLead",
    "handshakes",
    "maxQueuedBytes",
    "maxQueuedEvents",
    "physicalTransports",
    "queuedPayloadOwners",
    "starvedSubscriptions",
  ]);
  literal(document["activeTransportOwners"], 1);
  literal(document["currentPayloadOwners"], 0);
  literal(document["physicalTransports"], 1);
  literal(document["handshakes"], 1);
  integer(document["fairnessMaximumLead"], 0, 1);
  integer(document["maxQueuedBytes"], 0, E100_1K.maxDocumentBytes);
  integer(document["maxQueuedEvents"], 0, E100_1K.maxDocumentEvents);
  literal(document["starvedSubscriptions"], 0);
  literal(document["queuedPayloadOwners"], 0);
  const rustOwner = record(measurements["rustOwner"]);
  exact(rustOwner, [
    "dispatches",
    "fairnessMaximumLead",
    "finalCurrentSubscriptions",
    "logicalMemberships",
    "maxQueuedBytes",
    "maxQueuedEvents",
    "physicalDocumentTransports",
    "providerPath",
    "sequenceMismatches",
  ]);
  literal(rustOwner["providerPath"], "BoundedDocumentTransportSession");
  literal(rustOwner["physicalDocumentTransports"], 1);
  literal(rustOwner["logicalMemberships"], 100);
  literal(rustOwner["dispatches"], 1_100);
  literal(rustOwner["finalCurrentSubscriptions"], 100);
  literal(rustOwner["sequenceMismatches"], 0);
  integer(rustOwner["fairnessMaximumLead"], 0, 1);
  integer(rustOwner["maxQueuedEvents"], 0, E100_1K.maxDocumentEvents);
  integer(rustOwner["maxQueuedBytes"], 0, E100_1K.maxDocumentBytes);
  const subscriptions = array(measurements["subscriptions"], E100_1K.subscriptions);
  const seen = new Set<string>();
  const retainedById = new Map<string, number>();
  let presentations = 0;
  let refreshes = 0;
  for (const value of subscriptions) {
    const subscription = record(value);
    exact(subscription, [
      "current",
      "dispatches",
      "finalEpoch",
      "finalSequence",
      "id",
      "maxInFlightRefreshes",
      "maxQueuedRefreshes",
      "presentationEvents",
      "refreshInvalidations",
      "retention",
    ]);
    const id = text(subscription["id"]);
    if (!SUBSCRIPTION.test(id) || seen.has(id)) fail();
    seen.add(id);
    literal(subscription["current"], true);
    literal(subscription["dispatches"], 10);
    literal(subscription["presentationEvents"], 10);
    literal(subscription["refreshInvalidations"], 1);
    literal(subscription["finalEpoch"], "1");
    literal(subscription["finalSequence"], "11");
    integer(subscription["maxQueuedRefreshes"], 0, 1);
    integer(subscription["maxInFlightRefreshes"], 0, 1);
    const retainedBytes = retainedHeap(subscription["retention"]);
    retainedById.set(id, retainedBytes);
    presentations += integer(subscription["presentationEvents"]);
    refreshes += integer(subscription["refreshInvalidations"]);
  }
  if (presentations !== E100_1K.presentationEvents || refreshes !== E100_1K.refreshInvalidations) {
    fail();
  }
  void artifactSha256;
  return retainedById;
}

function r100(value: unknown, retainedById: ReadonlyMap<string, number>): void {
  const candidate = record(value);
  exact(candidate, ["bounds", "measurements", "workload"]);
  const workload = record(candidate["workload"]);
  exact(workload, ["reconnectHandshakes", "simultaneousContinuityLosses", "subscriptions"]);
  literal(workload["subscriptions"], 100);
  literal(workload["simultaneousContinuityLosses"], 100);
  literal(workload["reconnectHandshakes"], 1);
  const bounds = record(candidate["bounds"]);
  exact(bounds, [
    "maxConcurrentHandshakesPerOrigin",
    "maxRetainedBytesAfterCurrent",
    "reconnectHandshakes",
  ]);
  literal(bounds["maxConcurrentHandshakesPerOrigin"], 8);
  literal(bounds["maxRetainedBytesAfterCurrent"], 12_288);
  literal(bounds["reconnectHandshakes"], 1);
  const measurements = record(candidate["measurements"]);
  exact(measurements, ["document", "polling", "reconnectJitter", "recovery", "timeToCurrent"]);
  const document = record(measurements["document"]);
  exact(document, [
    "currentPayloadOwners",
    "generationAfter",
    "generationBefore",
    "maximumConcurrentReauthorizations",
    "physicalTransportsAfterCurrent",
    "predecessorContinuityOwners",
    "predecessorTransportOwners",
    "queuedPayloadOwners",
    "reconnectHandshakes",
    "recoveredSubscriptions",
    "starvedSubscriptions",
  ]);
  literal(document["generationBefore"], 1);
  literal(document["generationAfter"], 2);
  literal(document["reconnectHandshakes"], 1);
  literal(document["physicalTransportsAfterCurrent"], 1);
  literal(document["predecessorContinuityOwners"], 0);
  literal(document["predecessorTransportOwners"], 0);
  literal(document["queuedPayloadOwners"], 0);
  literal(document["currentPayloadOwners"], 0);
  integer(document["maximumConcurrentReauthorizations"], 1, 8);
  literal(document["recoveredSubscriptions"], 100);
  literal(document["starvedSubscriptions"], 0);
  const polling = record(measurements["polling"]);
  exact(polling, ["buckets", "maximumSameTick"]);
  literal(polling["maximumSameTick"], 1);
  const pollBuckets = array(polling["buckets"], 100);
  const pollDues = new Set<number>();
  for (const value of pollBuckets) {
    const bucket = record(value);
    exact(bucket, ["count", "dueMilliseconds"]);
    literal(bucket["count"], 1);
    const due = integer(bucket["dueMilliseconds"], 1);
    if (pollDues.has(due)) fail();
    pollDues.add(due);
  }
  const jitter = record(measurements["reconnectJitter"]);
  exact(jitter, ["buckets", "handshakes"]);
  literal(jitter["handshakes"], 1);
  const jitterBuckets = array(jitter["buckets"], 1);
  const jitterBucket = record(jitterBuckets[0]);
  exact(jitterBucket, ["count", "delayMilliseconds"]);
  literal(jitterBucket["count"], 1);
  integer(jitterBucket["delayMilliseconds"], 1);
  const recovery = array(measurements["recovery"], 100);
  const recoveryIds = new Set<string>();
  for (const value of recovery) {
    const entry = record(value);
    exact(entry, [
      "current",
      "id",
      "jitterMilliseconds",
      "pollDueMilliseconds",
      "retention",
      "timeToCurrentMilliseconds",
    ]);
    const id = text(entry["id"]);
    if (!SUBSCRIPTION.test(id) || recoveryIds.has(id)) fail();
    recoveryIds.add(id);
    literal(entry["current"], true);
    integer(entry["jitterMilliseconds"]);
    const pollDue = integer(entry["pollDueMilliseconds"], 1);
    if (!pollDues.has(pollDue)) fail();
    const retainedBytes = retainedHeap(entry["retention"]);
    if (!retainedById.has(id)) fail();
    void retainedBytes;
    finite(entry["timeToCurrentMilliseconds"]);
  }
  sampleSummary(measurements["timeToCurrent"], 100);
}

function multiDocument(value: unknown): void {
  const candidate = record(value);
  exact(candidate, [
    "attemptedHandshakes",
    "completedHandshakes",
    "documentCount",
    "label",
    "maximumConcurrentHandshakes",
    "origin",
    "startOrder",
  ]);
  literal(candidate["label"], "separate_multi_document_scheduler");
  literal(candidate["documentCount"], 16);
  literal(candidate["attemptedHandshakes"], 16);
  literal(candidate["completedHandshakes"], 16);
  integer(candidate["maximumConcurrentHandshakes"], 1, 8);
  text(candidate["origin"]);
  const order = array(candidate["startOrder"], 16).map((entry) => integer(entry, 0, 15));
  if (order.some((entry, index) => entry !== index)) fail();
}

function runs(value: unknown, independentRuns: number, artifactSha256: string): void {
  const entries = array(value, independentRuns);
  const processIds = new Set<number>();
  for (let index = 0; index < entries.length; index += 1) {
    const entry = record(entries[index]);
    exact(entry, [
      "artifactSha256",
      "dispatchP95Milliseconds",
      "evidenceSha256",
      "processId",
      "recoveryP95Milliseconds",
      "runIndex",
    ]);
    literal(entry["runIndex"], index + 1);
    literal(entry["artifactSha256"], artifactSha256);
    sha(entry["evidenceSha256"]);
    finite(entry["dispatchP95Milliseconds"]);
    finite(entry["recoveryP95Milliseconds"]);
    const processId = integer(entry["processId"], 1);
    if (processIds.has(processId)) fail();
    processIds.add(processId);
  }
}

function mutationProofs(value: unknown, artifactSha256: string): void {
  const candidate = record(value);
  exact(candidate, [
    "largeIslandBuffer",
    "predecessorTransport",
    "staleCurrentPayload",
    "staleQueuedPayload",
  ]);

  const largeIslandBuffer = record(candidate["largeIslandBuffer"]);
  exact(largeIslandBuffer, [
    "artifactSha256",
    "documentTransports",
    "phase",
    "retention",
    "subscriptionId",
  ]);
  literal(largeIslandBuffer["artifactSha256"], artifactSha256);
  literal(largeIslandBuffer["documentTransports"], 1);
  literal(largeIslandBuffer["phase"], "E100");
  literal(largeIslandBuffer["subscriptionId"], "subscription-000");
  if (retainedHeap(largeIslandBuffer["retention"]) <= E100_1K.maxRetainedBytesPerSubscription) {
    fail();
  }

  const predecessorTransport = record(candidate["predecessorTransport"]);
  exact(predecessorTransport, [
    "activeTransportOwners",
    "artifactSha256",
    "physicalTransportsAfterCurrent",
    "predecessorContinuityOwners",
    "predecessorTransportOwners",
    "reconnectHandshakes",
  ]);
  literal(predecessorTransport["artifactSha256"], artifactSha256);
  literal(predecessorTransport["activeTransportOwners"], 2);
  literal(predecessorTransport["physicalTransportsAfterCurrent"], 2);
  literal(predecessorTransport["predecessorContinuityOwners"], 100);
  literal(predecessorTransport["predecessorTransportOwners"], 1);
  literal(predecessorTransport["reconnectHandshakes"], 1);

  for (const [name, owner] of [
    ["staleCurrentPayload", "currentPayloadOwners"],
    ["staleQueuedPayload", "queuedPayloadOwners"],
  ] as const) {
    const stalePayload = record(candidate[name]);
    exact(stalePayload, ["artifactSha256", owner, "phase", "retention", "subscriptionId"]);
    literal(stalePayload["artifactSha256"], artifactSha256);
    literal(stalePayload[owner], 1);
    literal(stalePayload["phase"], "R100");
    literal(stalePayload["subscriptionId"], "subscription-000");
    if (retainedHeap(stalePayload["retention"]) <= R100.maxRetainedBytesAfterCurrent) fail();
  }
}

export function validateAsyncBudgetEvidence(value: unknown): AsyncBudgetEvidence {
  const candidate = record(value);
  exact(candidate, [
    "artifact",
    "e100",
    "environment",
    "methodology",
    "multiDocument",
    "mutationProofs",
    "r100",
    "recordedAt",
    "runs",
    "schemaVersion",
    "suite",
  ]);
  literal(candidate["schemaVersion"], 1);
  literal(candidate["suite"], "E100/1K+R100");
  const artifactRecord = record(candidate["artifact"]);
  artifact(artifactRecord);
  const artifactSha256 = sha(artifactRecord["sha256"]);
  const classification = environment(candidate["environment"]);
  const independentRuns = methodology(candidate["methodology"], classification);
  const retainedById = e100(candidate["e100"], artifactSha256);
  r100(candidate["r100"], retainedById);
  multiDocument(candidate["multiDocument"]);
  mutationProofs(candidate["mutationProofs"], artifactSha256);
  const recordedAt = text(candidate["recordedAt"]);
  if (!Number.isFinite(Date.parse(recordedAt))) fail();
  runs(candidate["runs"], independentRuns, artifactSha256);
  return value as AsyncBudgetEvidence;
}

export function validateAsyncBudgetBaseline(value: unknown): AsyncBudgetBaseline {
  const candidate = record(value);
  exact(candidate, ["exploratoryReference", "qualifiedBaseline", "schemaVersion", "suite"]);
  literal(candidate["schemaVersion"], 1);
  literal(candidate["suite"], "E100/1K+R100");
  const exploratory = validateAsyncBudgetEvidence(candidate["exploratoryReference"]);
  if (record(exploratory.environment)["classification"] !== "unqualified") fail();
  if (candidate["qualifiedBaseline"] !== null) {
    const qualified = validateAsyncBudgetEvidence(candidate["qualifiedBaseline"]);
    if (record(qualified.environment)["classification"] !== "qualified") fail();
  }
  return value as AsyncBudgetBaseline;
}

function median(values: readonly number[]): number {
  const ordered = [...values].sort((left, right) => left - right);
  const value = ordered[Math.floor(ordered.length / 2)];
  if (value === undefined) fail();
  return value;
}

function runMetric(evidence: AsyncBudgetEvidence, key: "dispatchP95Milliseconds"): number {
  return median(evidence.runs.map((entry) => finite(entry[key])));
}

function maximumRetainedBytes(evidence: AsyncBudgetEvidence, phase: "e100" | "r100"): number {
  const section = record(phase === "e100" ? evidence.e100 : evidence.r100);
  const measurements = record(section["measurements"]);
  const entries = array(
    phase === "e100" ? measurements["subscriptions"] : measurements["recovery"],
    100,
  );
  return Math.max(
    ...entries.map((entry) => integer(record(record(entry)["retention"])["retainedBytes"])),
  );
}

export function evaluateAsyncBudget(
  evidenceValue: unknown,
  baselineValue: unknown,
  options: Readonly<{ release: boolean }>,
): AsyncBudgetEvaluation {
  const evidence = validateAsyncBudgetEvidence(evidenceValue);
  const baseline = validateAsyncBudgetBaseline(baselineValue);
  const classification = record(evidence.environment)[
    "classification"
  ] as AsyncBudgetClassification;
  const issues: string[] = [];
  const observations: string[] = [];
  const e100Measurements = record(record(evidence.e100)["measurements"]);
  const dispatch = record(e100Measurements["dispatch"]);
  if (
    classification === "qualified" &&
    finite(dispatch["p95Milliseconds"]) > E100_1K.maxDispatchP95Milliseconds
  ) {
    issues.push("e100_dispatch_p95_exceeded");
  } else if (finite(dispatch["p95Milliseconds"]) > E100_1K.maxDispatchP95Milliseconds) {
    observations.push("e100_dispatch_p95_unqualified");
  }
  const e100RetainedBytes = maximumRetainedBytes(evidence, "e100");
  if (e100RetainedBytes > E100_1K.maxRetainedBytesPerSubscription) {
    if (classification === "qualified") issues.push("e100_retained_heap_exceeded");
    else observations.push("e100_retained_heap_unqualified");
  }
  const r100RetainedBytes = maximumRetainedBytes(evidence, "r100");
  if (r100RetainedBytes > R100.maxRetainedBytesAfterCurrent) {
    if (classification === "qualified") issues.push("r100_retained_heap_exceeded");
    else observations.push("r100_retained_heap_unqualified");
  }
  const qualifiedBaseline = baseline.qualifiedBaseline;
  if (qualifiedBaseline === null) observations.push("qualified_baseline_absent");
  if (options.release && qualifiedBaseline === null) issues.push("qualified_baseline_absent");
  if (classification !== "qualified") {
    if (options.release) issues.push("candidate_unqualified");
  } else if (qualifiedBaseline === null) {
    // Release mode already failed above; non-release qualified evidence remains observable.
  } else {
    const dispatchBaseline = runMetric(qualifiedBaseline, "dispatchP95Milliseconds");
    if (runMetric(evidence, "dispatchP95Milliseconds") >= dispatchBaseline * 1.15) {
      issues.push("e100_dispatch_regression");
    }
  }
  return Object.freeze({
    classification,
    issues: Object.freeze(issues),
    observations: Object.freeze(observations),
    status:
      issues.length > 0 ? "failed" : classification === "qualified" ? "passed" : "unqualified",
  });
}
