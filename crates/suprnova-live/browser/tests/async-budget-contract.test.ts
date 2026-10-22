import { access, readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  ASYNC_MULTI_DOCUMENT,
  E100_1K,
  R100,
  artifactDriftExceedsReviewThreshold,
  evaluateAsyncBudget,
  p95RegressionExceedsThreshold,
  qualifiedEnvironmentMatches,
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
      "run summary hides retained skew",
      (value) => {
        const run = required(value.runs[0]);
        run.e100RetainedMaximumBytes += 1;
      },
    ],
    [
      "forged E100 heap delta",
      (value) => (firstPostWorkload(value.e100.measurements.retention).usedSize += 1),
    ],
    [
      "forged R100 heap delta",
      (value) => (firstPostWorkload(value.r100.measurements.retention).usedSize += 1),
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
      "current payload survives cleanup",
      (value) => {
        const cleanup = value.r100.measurements.retention.cleanupResources;
        const first = required(cleanup.subscriptions[0]);
        first.currentPayloadBytes = 1_024;
        first.currentPayloadOwners = 1;
        cleanup.currentPayloadOwners = 1;
      },
    ],
    [
      "queued payload survives cleanup",
      (value) => {
        const cleanup = value.e100.measurements.retention.cleanupResources;
        const first = required(cleanup.subscriptions[0]);
        first.queuedPayloadBytes = 1_024;
        first.queuedPayloadOwners = 1;
        cleanup.queuedPayloadOwners = 1;
      },
    ],
    [
      "large-island retention mutation not detected",
      (value) => (value.mutationProofs.largeIslandBuffer.retention = retentionPhase()),
    ],
    [
      "predecessor transport mutation not detected",
      (value) =>
        (value.mutationProofs.predecessorTransport.retention.liveResources.predecessorTransportOwners = 0),
    ],
    [
      "stale current-payload mutation not detected",
      (value) =>
        (value.mutationProofs.staleCurrentPayload.retention.liveResources.currentPayloadOwners = 0),
    ],
    [
      "stale queued-payload mutation not detected",
      (value) =>
        (value.mutationProofs.staleQueuedPayload.retention.liveResources.queuedPayloadOwners = 0),
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

  it("keeps artifact review and p95 regression boundaries distinct", () => {
    expect(artifactDriftExceedsReviewThreshold(1_149, 1_000)).toBe(false);
    expect(artifactDriftExceedsReviewThreshold(1_150, 1_000)).toBe(false);
    expect(artifactDriftExceedsReviewThreshold(1_151, 1_000)).toBe(true);

    expect(p95RegressionExceedsThreshold(1.149, 1)).toBe(false);
    expect(p95RegressionExceedsThreshold(1.15, 1)).toBe(true);
    expect(p95RegressionExceedsThreshold(1.151, 1)).toBe(true);

    const reference = validEvidence();
    reference.artifact.brotliBytes = 1_000;
    const baseline = {
      exploratoryReference: reference,
      qualifiedBaseline: null,
      schemaVersion: 1 as const,
      suite: "E100/1K+R100" as const,
    };
    const exactThreshold = validEvidence();
    exactThreshold.artifact.brotliBytes = 1_150;
    expect(evaluateAsyncBudget(exactThreshold, baseline, { release: false }).issues).not.toContain(
      "async_artifact_review_required",
    );
    const aboveThreshold = validEvidence();
    aboveThreshold.artifact.brotliBytes = 1_151;
    expect(evaluateAsyncBudget(aboveThreshold, baseline, { release: false }).issues).toContain(
      "async_artifact_review_required",
    );
  });

  it("fails qualified comparison closed when the B1 environment identity changes", () => {
    const baselineEvidence = qualifiedEvidence([1, 1, 1]);
    const baseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline: baselineEvidence,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    for (const key of [
      "browserRevision",
      "playwrightVersion",
      "cpuModel",
      "kernel",
      "governor",
    ] as const) {
      const changedIdentity = structuredClone(baselineEvidence.environment);
      changedIdentity[key] = `${changedIdentity[key]}-changed`;
      expect(qualifiedEnvironmentMatches(changedIdentity, baselineEvidence.environment), key).toBe(
        false,
      );
      if (key === "governor") continue;
      const candidate = qualifiedEvidence([2, 2, 2]);
      candidate.environment[key] = `${candidate.environment[key]}-changed`;
      const evaluation = evaluateAsyncBudget(candidate, baseline, { release: true });
      expect(evaluation.issues, key).toContain("qualified_environment_mismatch");
      expect(evaluation.issues, key).not.toContain("e100_dispatch_regression");
    }

    const changedArtifact = qualifiedEvidence([2, 2, 2]);
    rebindArtifact(changedArtifact, "e".repeat(64));
    const artifactEvaluation = evaluateAsyncBudget(changedArtifact, baseline, { release: true });
    expect(artifactEvaluation.issues).toContain("qualified_artifact_mismatch");
    expect(artifactEvaluation.issues).not.toContain("e100_dispatch_regression");
  });

  it("keeps a powersave governor unqualified", () => {
    const candidate = validEvidence();
    expect(candidate.environment.governor).toBe("powersave");
    expect(validateAsyncBudgetEvidence(candidate).environment["classification"]).toBe(
      "unqualified",
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
    addOwnedRetention(exploratory, "e100", 9_000);
    const baseline = {
      exploratoryReference: validEvidence(),
      qualifiedBaseline: null,
      schemaVersion: 1,
      suite: "E100/1K+R100",
    };
    expect(evaluateAsyncBudget(exploratory, baseline, { release: false }).observations).toContain(
      "e100_retained_heap_unqualified",
    );
    addOwnedRetention(exploratory, "r100", 13_000);
    expect(evaluateAsyncBudget(exploratory, baseline, { release: false }).observations).toContain(
      "r100_retained_heap_unqualified",
    );

    const qualified = qualifiedEvidence([1, 1, 1]);
    addOwnedRetention(qualified, "e100", 9_000);
    expect(evaluateAsyncBudget(qualified, baseline, { release: true }).issues).toContain(
      "e100_retained_heap_exceeded",
    );
    addOwnedRetention(qualified, "r100", 13_000);
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

function retentionPhase(
  options: Readonly<{
    activeTransportOwners?: number;
    currentPayloadBytes?: number;
    currentPayloadOwners?: number;
    predecessorContinuityOwners?: number;
    predecessorTransportOwners?: number;
    queuedPayloadBytes?: number;
    queuedPayloadOwners?: number;
  }> = {},
) {
  const baseline = Array.from({ length: 5 }, () => ({
    backingStorageSize: 20_000,
    embedderHeapUsedSize: 30_000,
    usedSize: 1_000_000,
  }));
  const subscriptions = Array.from({ length: 100 }, (_, index) => ({
    authorizationBytes: 835,
    currentPayloadBytes: index === 0 ? (options.currentPayloadBytes ?? 0) : 0,
    currentPayloadOwners: index === 0 ? (options.currentPayloadOwners ?? 0) : 0,
    id: `subscription-${String(index).padStart(3, "0")}`,
    queuedPayloadBytes: index === 0 ? (options.queuedPayloadBytes ?? 0) : 0,
    queuedPayloadOwners: index === 0 ? (options.queuedPayloadOwners ?? 0) : 0,
  }));
  const totalOwnedBytes = subscriptions.reduce(
    (sum, entry) =>
      sum + entry.authorizationBytes + entry.currentPayloadBytes + entry.queuedPayloadBytes,
    0,
  );
  const sharedStructuralBytes = 216_500;
  const totalRetainedBytes = totalOwnedBytes + sharedStructuralBytes;
  const postWorkload = Array.from({ length: 5 }, () => ({
    backingStorageSize: 20_000,
    embedderHeapUsedSize: 30_000,
    usedSize: 1_000_000 + totalRetainedBytes,
  }));
  const cleanup = structuredClone(baseline);
  const sharedAmortizedBytes = Math.ceil(sharedStructuralBytes / 100);
  const derivedSubscriptions = subscriptions.map((entry) => {
    const ownedBytes =
      entry.authorizationBytes + entry.currentPayloadBytes + entry.queuedPayloadBytes;
    return { ...entry, ownedBytes, retainedBytes: sharedAmortizedBytes + ownedBytes };
  });
  const resource = {
    activeTransportOwners: options.activeTransportOwners ?? 1,
    currentPayloadOwners: options.currentPayloadOwners ?? 0,
    predecessorContinuityOwners: options.predecessorContinuityOwners ?? 0,
    predecessorTransportOwners: options.predecessorTransportOwners ?? 0,
    queuedPayloadOwners: options.queuedPayloadOwners ?? 0,
    subscriptions,
  };
  const cleanupSubscriptions = subscriptions.map((entry) => ({
    ...entry,
    authorizationBytes: 0,
    currentPayloadBytes: 0,
    currentPayloadOwners: 0,
    queuedPayloadBytes: 0,
    queuedPayloadOwners: 0,
  }));
  return {
    baseline,
    cleanup,
    cleanupResidualBytes: 0,
    cleanupResources: {
      activeTransportOwners: 0,
      currentPayloadOwners: 0,
      predecessorContinuityOwners: 0,
      predecessorTransportOwners: 0,
      queuedPayloadOwners: 0,
      subscriptions: cleanupSubscriptions,
    },
    liveResources: resource,
    postWorkload,
    sharedAmortizedBytes,
    sharedStructuralBytes,
    subscriptions: derivedSubscriptions,
    totalOwnedBytes,
    totalRetainedBytes,
  };
}

function validEvidence() {
  const dispatchDurations: number[] = Array.from({ length: 1_000 }, (_, index) =>
    index < 500 ? 0.5 : index < 949 ? 0.75 : 1,
  );
  const e100Retention = retentionPhase();
  const r100Retention = retentionPhase();
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
    retention: required(e100Retention.subscriptions[index]),
  }));
  const recovery = Array.from({ length: 100 }, (_, index) => {
    const subscription = subscriptions[index];
    if (subscription === undefined) throw new Error("fixture_subscription_missing");
    return {
      current: true,
      id: `subscription-${String(index).padStart(3, "0")}`,
      jitterMilliseconds: index + 1,
      pollDueMilliseconds: 30_001 + index,
      retention: required(r100Retention.subscriptions[index]),
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
        retention: e100Retention,
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
      governor: "powersave",
      kernel: "fixture",
      memoryBytes: 32 * 1_024 * 1_024 * 1_024,
      operatingSystem: "linux",
      playwrightVersion: "1.62.1",
      profile: "B1",
      providerProfile: "rust-owner-browser-measured-source-v1",
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
        baselineState: "same-page controller baseline",
        cleanupState: "all original controllers disposed and owners released",
        derivation:
          "total=max(post_workload)-min(baseline); shared=max(0,total-actual_owner_bytes); per_island=ceil(shared/100)+actual_owner_bytes",
        exclusions: ["DOM", "benchmark_harness", "released_current_payload"],
        garbageCollection: "HeapProfiler.collectGarbage",
        harnessTreatment: "same harness in baseline and post-workload",
        phaseSamples: 5,
        postWorkloadState: "all 100 original workloaded controllers remain live and current",
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
        productPath: "AsyncSubscription.pending",
        retention: retentionPhase({ queuedPayloadBytes: 65_536, queuedPayloadOwners: 1 }),
        subscriptionId: "subscription-000",
      },
      predecessorTransport: {
        artifactSha256: "a".repeat(64),
        physicalTransportsAfterCurrent: 2,
        productPath: "AsyncDocumentOwner.transport",
        reconnectHandshakes: 1,
        retention: retentionPhase({
          activeTransportOwners: 2,
          predecessorContinuityOwners: 100,
          predecessorTransportOwners: 1,
        }),
      },
      staleCurrentPayload: {
        artifactSha256: "a".repeat(64),
        phase: "R100",
        productPath: "AsyncSubscription.activeRefresh",
        retention: retentionPhase({ currentPayloadBytes: 1_024, currentPayloadOwners: 1 }),
        subscriptionId: "subscription-000",
      },
      staleQueuedPayload: {
        artifactSha256: "a".repeat(64),
        phase: "R100",
        productPath: "AsyncSubscription.pending",
        retention: retentionPhase({ queuedPayloadBytes: 65_536, queuedPayloadOwners: 1 }),
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
        retention: r100Retention,
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
        e100RetainedMaximumBytes: Math.max(
          ...e100Retention.subscriptions.map((entry) => entry.retainedBytes),
        ),
        evidenceSha256: "d".repeat(64),
        processId: 123,
        r100RetainedMaximumBytes: Math.max(
          ...r100Retention.subscriptions.map((entry) => entry.retainedBytes),
        ),
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

function required<Value>(value: Value | undefined): Value {
  if (value === undefined) throw new Error("fixture_value_missing");
  return value;
}

function firstPostWorkload(retention: ReturnType<typeof retentionPhase>) {
  return required(retention.postWorkload[0]);
}

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

function qualifiedEvidence(dispatchP95: readonly [number, number, number]): MutableEvidence {
  const evidence = validEvidence();
  evidence.environment.classification = "qualified";
  evidence.environment.dedicatedVcpusAttested = true;
  evidence.environment.governor = "performance";
  evidence.environment.qualificationRequirementsMet = true;
  evidence.environment.selectedCpuCount = 8;
  evidence.methodology.independentRuns = 3;
  evidence.runs = dispatchP95.map((dispatchP95Milliseconds, index) => ({
    artifactSha256: "a".repeat(64),
    dispatchP95Milliseconds,
    e100RetainedMaximumBytes: Math.max(
      ...evidence.e100.measurements.retention.subscriptions.map((entry) => entry.retainedBytes),
    ),
    evidenceSha256: String(index + 1).repeat(64),
    processId: 10_000 + index,
    r100RetainedMaximumBytes: Math.max(
      ...evidence.r100.measurements.retention.subscriptions.map((entry) => entry.retainedBytes),
    ),
    recoveryP95Milliseconds: 1,
    runIndex: index + 1,
  }));
  return evidence;
}

function addOwnedRetention(evidence: MutableEvidence, phase: "e100" | "r100", bytes: number): void {
  const retention =
    phase === "e100" ? evidence.e100.measurements.retention : evidence.r100.measurements.retention;
  const live = retention.liveResources.subscriptions[0];
  const derived = retention.subscriptions[0];
  if (live === undefined || derived === undefined) throw new Error("fixture_subscription_missing");
  live.authorizationBytes += bytes;
  derived.authorizationBytes += bytes;
  derived.ownedBytes += bytes;
  derived.retainedBytes += bytes;
  retention.totalOwnedBytes += bytes;
  retention.totalRetainedBytes += bytes;
  for (const sample of retention.postWorkload) sample.usedSize += bytes;
  if (phase === "e100") {
    firstSubscription(evidence).retention = derived;
    for (const run of evidence.runs) run.e100RetainedMaximumBytes = derived.retainedBytes;
  } else {
    firstRecovery(evidence).retention = derived;
    for (const run of evidence.runs) run.r100RetainedMaximumBytes = derived.retainedBytes;
  }
}

function rebindArtifact(evidence: MutableEvidence, artifactSha256: string): void {
  evidence.artifact.sha256 = artifactSha256;
  for (const run of evidence.runs) run.artifactSha256 = artifactSha256;
  evidence.mutationProofs.largeIslandBuffer.artifactSha256 = artifactSha256;
  evidence.mutationProofs.predecessorTransport.artifactSha256 = artifactSha256;
  evidence.mutationProofs.staleCurrentPayload.artifactSha256 = artifactSha256;
  evidence.mutationProofs.staleQueuedPayload.artifactSha256 = artifactSha256;
}

function setDispatchP95(evidence: MutableEvidence, milliseconds: number): void {
  evidence.e100.measurements.dispatch.durationsMilliseconds.fill(milliseconds);
  evidence.e100.measurements.dispatch.p50Milliseconds = milliseconds;
  evidence.e100.measurements.dispatch.p95Milliseconds = milliseconds;
  for (const run of evidence.runs) run.dispatchP95Milliseconds = milliseconds;
}
