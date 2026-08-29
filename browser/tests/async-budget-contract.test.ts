import { access, readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  ASYNC_MULTI_DOCUMENT,
  E100_1K,
  R100,
  evaluateAsyncBudget,
  validateAsyncBudgetBaseline,
  validateAsyncBudgetEvidence,
} from "../benchmarks/async-budget-schema.js";

const REQUIRED_FILES = Object.freeze([
  "benchmarks/async-budget-schema.ts",
  "benchmarks/async-budget-workloads.ts",
  "benchmarks/baselines/async-budget-v1.json",
  "scripts/run-async-budget.mjs",
  "../scripts/run-async-budget.sh",
]);

describe("E100/1K and R100 hard-budget wiring", () => {
  it("ships the dedicated workload, schema, baseline, and unattended runner", async () => {
    await expect(Promise.all(REQUIRED_FILES.map((path) => access(path)))).resolves.toBeDefined();

    const packageJson = JSON.parse(await readFile("package.json", "utf8")) as {
      scripts?: Readonly<Record<string, string>>;
    };
    expect(packageJson.scripts?.["budget:async"]).toBe("node scripts/run-async-budget.mjs");
    expect(
      validateAsyncBudgetBaseline(
        JSON.parse(await readFile("benchmarks/baselines/async-budget-v1.json", "utf8")) as unknown,
      ),
    ).toBeDefined();
  });

  it("locks both workload IDs, their exact topology, and every hard bound", () => {
    expect(E100_1K).toEqual({
      subscriptions: 100,
      documentTransports: 1,
      presentationEvents: 1_000,
      payloadBytes: 1_024,
      durationMs: 10_000,
      refreshInvalidations: 100,
      maxRetainedBytesPerSubscription: 8_192,
      maxDocumentEvents: 64,
      maxDocumentBytes: 262_144,
      maxDispatchP95Milliseconds: 8,
      maxQueuedRefreshesPerIsland: 1,
      maxInFlightRefreshesPerIsland: 1,
    });
    expect(R100).toEqual({
      subscriptions: 100,
      reconnectHandshakes: 1,
      maxConcurrentHandshakesPerOrigin: 8,
      maxRetainedBytesAfterCurrent: 12_288,
    });
    expect(ASYNC_MULTI_DOCUMENT).toEqual({
      documents: 16,
      attemptedHandshakes: 16,
      maxConcurrentHandshakesPerOrigin: 8,
    });
  });

  it("accepts strict reduced exploratory evidence and fails closed for release", () => {
    const evidence = validEvidence();
    expect(validateAsyncBudgetEvidence(evidence)).toEqual(evidence);
    const baseline = {
      exploratoryReference: evidence,
      qualifiedBaseline: null,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    expect(validateAsyncBudgetBaseline(baseline)).toEqual(baseline);
    expect(evaluateAsyncBudget(evidence, baseline, { release: false })).toEqual({
      classification: "unqualified",
      issues: [],
      observations: ["qualified_baseline_absent"],
      status: "unqualified",
    });
    const release = evaluateAsyncBudget(evidence, baseline, { release: true });
    expect(release.status).toBe("failed");
    expect(release.issues).toContain("candidate_unqualified");
    expect(release.issues).toContain("qualified_baseline_absent");
  });

  const mutations: readonly (readonly [string, (value: MutableEvidence) => void])[] = [
    ["chatty island", (value) => (firstSubscription(value).presentationEvents = 11)],
    ["queue overflow", (value) => (value.e100.measurements.document.maxQueuedEvents = 65)],
    [
      "two physical transports",
      (value) => (value.e100.measurements.document.physicalTransports = 2),
    ],
    [
      "two reconnect handshakes",
      (value) => (value.r100.measurements.document.reconnectHandshakes = 2),
    ],
    ["synchronized polls", (value) => (value.r100.measurements.polling.maximumSameTick = 2)],
    ["nine active handshakes", (value) => (value.multiDocument.maximumConcurrentHandshakes = 9)],
    ["duplicate refresh", (value) => (firstSubscription(value).refreshInvalidations = 2)],
    [
      "forged E100 heap delta",
      (value) => (firstAfter(firstSubscription(value).retention).usedSize += 1),
    ],
    [
      "forged R100 heap delta",
      (value) => (firstAfter(firstRecovery(value).retention).usedSize += 1),
    ],
    [
      "predecessor transport retained",
      (value) => (value.r100.measurements.document.predecessorTransportOwners = 1),
    ],
    [
      "predecessor continuity retained",
      (value) => (value.r100.measurements.document.predecessorContinuityOwners = 1),
    ],
    [
      "stale current payload retained",
      (value) => (value.r100.measurements.document.currentPayloadOwners = 1),
    ],
    [
      "stale queued payload retained",
      (value) => (value.r100.measurements.document.queuedPayloadOwners = 1),
    ],
    [
      "large-island retention mutation not detected",
      (value) => (value.mutationProofs.largeIslandBuffer.retention = heapRetention(8_192)),
    ],
    [
      "predecessor transport mutation not detected",
      (value) => (value.mutationProofs.predecessorTransport.predecessorTransportOwners = 0),
    ],
    [
      "stale current-payload mutation not detected",
      (value) => (value.mutationProofs.staleCurrentPayload.currentPayloadOwners = 0),
    ],
    [
      "stale queued-payload mutation not detected",
      (value) => (value.mutationProofs.staleQueuedPayload.queuedPayloadOwners = 0),
    ],
  ];

  it("rejects every required topology and resource mutation", () => {
    for (const [, mutate] of mutations) {
      const evidence = structuredClone(validEvidence());
      mutate(evidence);
      expect(() => validateAsyncBudgetEvidence(evidence)).toThrow("async_budget_evidence_invalid");
    }
  });

  it("uses independent-process median p95 and fails at a 15-percent regression", () => {
    const exploratory = validEvidence();
    const qualifiedBaseline = qualifiedEvidence([1, 2, 3]);
    const baseline = {
      exploratoryReference: exploratory,
      qualifiedBaseline,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    const below = qualifiedEvidence([3, 2.299_999, 1]);
    expect(evaluateAsyncBudget(below, baseline, { release: true }).issues).not.toContain(
      "e100_dispatch_regression",
    );
    const threshold = qualifiedEvidence([1, 3, 2.3]);
    expect(evaluateAsyncBudget(threshold, baseline, { release: true }).issues).toContain(
      "e100_dispatch_regression",
    );
    const permuted = qualifiedEvidence([3, 2.3, 1]);
    expect(evaluateAsyncBudget(permuted, baseline, { release: true }).issues).toContain(
      "e100_dispatch_regression",
    );

    const duplicateProcess = qualifiedEvidence([1, 2, 3]);
    const firstRun = duplicateProcess.runs[0];
    const secondRun = duplicateProcess.runs[1];
    if (firstRun === undefined || secondRun === undefined) throw new Error("fixture_run_missing");
    secondRun.processId = firstRun.processId;
    expect(() => validateAsyncBudgetEvidence(duplicateProcess)).toThrow(
      "async_budget_evidence_invalid",
    );
  });

  it("records R100 recovery timing without inventing a latency regression gate", () => {
    const candidate = qualifiedEvidence([1, 1, 1]);
    for (const run of candidate.runs) run.recoveryP95Milliseconds = 100;
    const baseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline: qualifiedEvidence([1, 1, 1]),
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };

    expect(evaluateAsyncBudget(candidate, baseline, { release: true })).toEqual({
      classification: "qualified",
      issues: [],
      observations: [],
      status: "passed",
    });
  });

  it("fails one measured island over the cap even when the document average remains small", () => {
    const exploratory = validEvidence();
    const first = firstSubscription(exploratory);
    firstAfter(first.retention).usedSize += 9_000;
    first.retention.retainedBytes += 9_000;
    const baseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline: null,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    expect(evaluateAsyncBudget(exploratory, baseline, { release: false }).observations).toContain(
      "e100_retained_heap_unqualified",
    );
    const recovered = firstRecovery(exploratory);
    firstAfter(recovered.retention).usedSize += 13_000;
    recovered.retention.retainedBytes += 13_000;
    expect(evaluateAsyncBudget(exploratory, baseline, { release: false }).observations).toContain(
      "r100_retained_heap_unqualified",
    );

    const qualified = qualifiedEvidence([1, 1, 1]);
    const qualifiedFirst = firstSubscription(qualified);
    firstAfter(qualifiedFirst.retention).usedSize += 9_000;
    qualifiedFirst.retention.retainedBytes += 9_000;
    expect(evaluateAsyncBudget(qualified, baseline, { release: true }).issues).toContain(
      "e100_retained_heap_exceeded",
    );
    const qualifiedRecovered = firstRecovery(qualified);
    firstAfter(qualifiedRecovered.retention).usedSize += 13_000;
    qualifiedRecovered.retention.retainedBytes += 13_000;
    expect(evaluateAsyncBudget(qualified, baseline, { release: true }).issues).toContain(
      "r100_retained_heap_exceeded",
    );
  });

  it("reports an unqualified cap observation but never weakens the qualified release cap", () => {
    const exploratory = validEvidence();
    setDispatchP95(exploratory, 8.3);
    const qualifiedBaseline = qualifiedEvidence([1, 1, 1]);
    const baseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    const exploratoryEvaluation = evaluateAsyncBudget(exploratory, baseline, { release: false });
    expect(exploratoryEvaluation.observations).toContain("e100_dispatch_p95_unqualified");
    expect(exploratoryEvaluation.issues).not.toContain("e100_dispatch_p95_exceeded");

    const qualified = qualifiedEvidence([8.3, 8.3, 8.3]);
    setDispatchP95(qualified, 8.3);
    const qualifiedEvaluation = evaluateAsyncBudget(qualified, baseline, { release: true });
    expect(qualifiedEvaluation.issues).toContain("e100_dispatch_p95_exceeded");

    const noQualifiedBaseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline: null,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    const withinCap = qualifiedEvidence([1, 1, 1]);
    expect(evaluateAsyncBudget(withinCap, noQualifiedBaseline, { release: true }).issues).toContain(
      "qualified_baseline_absent",
    );
  });
});

function heapRetention(retainedBytes: number) {
  const before = Array.from({ length: 5 }, () => ({
    backingStorageSize: 20_000,
    embedderHeapUsedSize: 30_000,
    usedSize: 1_000_000,
  }));
  const after = Array.from({ length: 5 }, () => ({
    backingStorageSize: 20_000,
    embedderHeapUsedSize: 30_000,
    usedSize: 1_000_000 + retainedBytes,
  }));
  return { after, before, retainedBytes };
}

function validEvidence() {
  const dispatchDurations: number[] = Array.from({ length: 1_000 }, (_, index) =>
    index < 500 ? 0.5 : index < 949 ? 0.75 : 1,
  );
  const subscriptions = Array.from({ length: 100 }, (_, index) => ({
    current: true,
    dispatches: 10,
    finalEpoch: "1",
    finalSequence: "11",
    id: `subscription-${String(index).padStart(3, "0")}`,
    maxInFlightRefreshes: 1,
    maxQueuedRefreshes: 1,
    presentationEvents: 10,
    refreshInvalidations: 1,
    retention: heapRetention(2_176 + index),
  }));
  const recovery = Array.from({ length: 100 }, (_, index) => {
    const subscription = subscriptions[index];
    if (subscription === undefined) throw new Error("fixture_subscription_missing");
    return {
      current: true,
      id: `subscription-${String(index).padStart(3, "0")}`,
      jitterMilliseconds: index + 1,
      pollDueMilliseconds: 30_001 + index,
      retention: heapRetention(2_176 + index),
      timeToCurrentMilliseconds: 1 + index / 100,
    };
  });
  const evidence = {
    artifact: {
      brotliBytes: 18_713,
      file: "suprnova-live.async.esm.js",
      manifestSha256: "b".repeat(64),
      role: "async-esm",
      sha256: "a".repeat(64),
      sourceInputsSha256: "c".repeat(64),
    },
    e100: {
      bounds: {
        maxDispatchP95Milliseconds: 8,
        maxDocumentBytes: 262_144,
        maxDocumentEvents: 64,
        maxInFlightRefreshesPerIsland: 1,
        maxQueuedRefreshesPerIsland: 1,
        maxRetainedBytesPerSubscription: 8_192,
      },
      measurements: {
        dispatch: {
          durationsMilliseconds: dispatchDurations,
          p50Milliseconds: 0.5,
          p95Milliseconds: 1,
          sampleCount: 1_000,
        },
        document: {
          activeTransportOwners: 1,
          currentPayloadOwners: 0,
          fairnessMaximumLead: 1,
          handshakes: 1,
          maxQueuedBytes: 32_768,
          maxQueuedEvents: 32,
          physicalTransports: 1,
          queuedPayloadOwners: 0,
          starvedSubscriptions: 0,
        },
        rustOwner: {
          dispatches: 1_100,
          fairnessMaximumLead: 1,
          finalCurrentSubscriptions: 100,
          logicalMemberships: 100,
          maxQueuedBytes: 65_536,
          maxQueuedEvents: 64,
          physicalDocumentTransports: 1,
          providerPath: "BoundedDocumentTransportSession",
          sequenceMismatches: 0,
        },
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
    environment: {
      architecture: "x86_64",
      browser: "chromium",
      browserRevision: "1234",
      classification: "unqualified",
      cpuModel: "fixture",
      cpuThrottleRate: 4,
      dedicatedVcpusAttested: false,
      extensions: false,
      kernel: "fixture",
      memoryBytes: 32 * 1_024 * 1_024 * 1_024,
      operatingSystem: "linux",
      playwrightVersion: "1.62.1",
      profile: "B1",
      qualificationRequirementsMet: false,
      selectedCpuCount: 8,
      viewport: { height: 720, width: 1_280 },
      warmHttpCache: true,
    },
    methodology: {
      controlledTimeline: true,
      independentRuns: 1,
      measuredSamples: 1_000,
      monotonicClock: "performance.now",
      retainedHeap: {
        api: "Chromium CDP Runtime.getHeapUsage",
        beforeState:
          "same page, DOM, benchmark harness, and native document transport with target island disconnected",
        derivation: "max_after_total_minus_min_before_total",
        exclusions: ["native_transport", "DOM", "current_payload"],
        garbageCollection: "HeapProfiler.collectGarbage",
        harnessTreatment:
          "control harness is retained in both states; connected controller and port are conservatively included",
        phaseSamples: 5,
        product: "Chrome/fixture",
        protocolVersion: "1.3",
        unavailable: "fail_closed",
      },
      regressionReference: "median_run_p95_v1",
      warmupIterations: 1,
      watchdogOutsideSamples: true,
    },
    multiDocument: {
      attemptedHandshakes: 16,
      completedHandshakes: 16,
      documentCount: 16,
      label: "separate_multi_document_scheduler",
      maximumConcurrentHandshakes: 8,
      origin: "http://127.0.0.1:4173",
      startOrder: Array.from({ length: 16 }, (_, index) => index),
    },
    mutationProofs: {
      largeIslandBuffer: {
        artifactSha256: "a".repeat(64),
        documentTransports: 1,
        phase: "E100",
        retention: heapRetention(65_536),
        subscriptionId: "subscription-000",
      },
      predecessorTransport: {
        activeTransportOwners: 2,
        artifactSha256: "a".repeat(64),
        physicalTransportsAfterCurrent: 2,
        predecessorContinuityOwners: 100,
        predecessorTransportOwners: 1,
        reconnectHandshakes: 1,
      },
      staleCurrentPayload: {
        artifactSha256: "a".repeat(64),
        currentPayloadOwners: 1,
        phase: "R100",
        retention: heapRetention(65_536),
        subscriptionId: "subscription-000",
      },
      staleQueuedPayload: {
        artifactSha256: "a".repeat(64),
        phase: "R100",
        queuedPayloadOwners: 1,
        retention: heapRetention(65_536),
        subscriptionId: "subscription-000",
      },
    },
    recordedAt: "2026-08-29T00:00:00.000Z",
    r100: {
      bounds: {
        maxConcurrentHandshakesPerOrigin: 8,
        maxRetainedBytesAfterCurrent: 12_288,
        reconnectHandshakes: 1,
      },
      measurements: {
        document: {
          currentPayloadOwners: 0,
          generationAfter: 2,
          generationBefore: 1,
          maximumConcurrentReauthorizations: 8,
          physicalTransportsAfterCurrent: 1,
          predecessorContinuityOwners: 0,
          predecessorTransportOwners: 0,
          queuedPayloadOwners: 0,
          reconnectHandshakes: 1,
          recoveredSubscriptions: 100,
          starvedSubscriptions: 0,
        },
        polling: {
          buckets: recovery.map(({ pollDueMilliseconds }) => ({
            count: 1,
            dueMilliseconds: pollDueMilliseconds,
          })),
          maximumSameTick: 1,
        },
        reconnectJitter: {
          buckets: [{ count: 1, delayMilliseconds: 250 }],
          handshakes: 1,
        },
        recovery,
        timeToCurrent: {
          durationsMilliseconds: recovery.map(
            ({ timeToCurrentMilliseconds }) => timeToCurrentMilliseconds,
          ),
          p50Milliseconds: 1.49,
          p95Milliseconds: 1.94,
          sampleCount: 100,
        },
      },
      workload: {
        reconnectHandshakes: 1,
        simultaneousContinuityLosses: 100,
        subscriptions: 100,
      },
    },
    runs: [
      {
        artifactSha256: "a".repeat(64),
        dispatchP95Milliseconds: 1,
        evidenceSha256: "d".repeat(64),
        processId: 123,
        recoveryP95Milliseconds: 1.94,
        runIndex: 1,
      },
    ],
    schemaVersion: 1,
    suite: "E100/1K+R100",
  };
  return evidence;
}

type MutableEvidence = ReturnType<typeof validEvidence>;

function firstSubscription(value: MutableEvidence) {
  const first = value.e100.measurements.subscriptions[0];
  if (first === undefined) throw new Error("fixture_subscription_missing");
  return first;
}

function firstRecovery(value: MutableEvidence) {
  const first = value.r100.measurements.recovery[0];
  if (first === undefined) throw new Error("fixture_recovery_missing");
  return first;
}

function firstAfter(retention: ReturnType<typeof heapRetention>) {
  const first = retention.after[0];
  if (first === undefined) throw new Error("fixture_heap_sample_missing");
  return first;
}

function qualifiedEvidence(dispatchP95: readonly [number, number, number]): MutableEvidence {
  const evidence = validEvidence();
  evidence.environment.classification = "qualified";
  evidence.environment.dedicatedVcpusAttested = true;
  evidence.environment.qualificationRequirementsMet = true;
  evidence.environment.selectedCpuCount = 8;
  evidence.methodology.independentRuns = 3;
  evidence.runs = dispatchP95.map((dispatchP95Milliseconds, index) => ({
    artifactSha256: "a".repeat(64),
    dispatchP95Milliseconds,
    evidenceSha256: String(index + 1).repeat(64),
    processId: 10_000 + index,
    recoveryP95Milliseconds: 1,
    runIndex: index + 1,
  }));
  return evidence;
}

function setDispatchP95(evidence: MutableEvidence, milliseconds: number): void {
  evidence.e100.measurements.dispatch.durationsMilliseconds.fill(milliseconds);
  evidence.e100.measurements.dispatch.p50Milliseconds = milliseconds;
  evidence.e100.measurements.dispatch.p95Milliseconds = milliseconds;
  for (const run of evidence.runs) run.dispatchP95Milliseconds = milliseconds;
}
