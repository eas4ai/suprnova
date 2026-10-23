import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  classifyBenchmarkEnvironment,
  evaluateBrowserBudget,
  validateBrowserBudgetResult,
} from "../benchmarks/schema.js";
import { classifyP95Regression, summarizeSamples } from "../benchmarks/statistics.js";
import {
  createD100Workload,
  createE100Workload,
  createMorphWorkload,
  createR100Workload,
} from "../benchmarks/workloads.js";

describe("browser performance evidence contract", () => {
  it("generates the exact D100, M1K, M5K, E100/1K, and R100 workload shapes", () => {
    const d100 = createD100Workload();
    expect(Buffer.byteLength(d100.html, "utf8")).toBe(64 * 1024);
    expect(d100.documentBytes).toBe(64 * 1024);
    expect(d100.islandCount).toBe(100);
    expect(d100.html.match(/data-suprnova-live-island/gu)).toHaveLength(100);

    const m1k = createMorphWorkload("M1K");
    expect(m1k.elementCount).toBe(1_000);
    expect(m1k.maximumDepth).toBe(12);
    expect(m1k.changedNodeCount).toBe(100);
    expect(m1k.sourceHtml.match(/data-suprnova-live-key=/gu)).toHaveLength(1_000);
    expect(m1k.targetHtml.match(/data-suprnova-live-key=/gu)).toHaveLength(1_000);

    const m5k = createMorphWorkload("M5K");
    expect(m5k.elementCount).toBe(5_000);
    expect(m5k.maximumDepth).toBe(24);
    expect(m5k.changedNodeCount).toBe(500);
    expect(m5k.sourceHtml.match(/data-suprnova-live-key=/gu)).toHaveLength(5_000);
    expect(m5k.targetHtml.match(/data-suprnova-live-key=/gu)).toHaveLength(5_000);

    expect(createE100Workload()).toMatchObject({
      id: "E100/1K",
      subscriptionCount: 100,
      presentationEventCount: 1_000,
      eventEnvelopeBytes: 1_024,
      scheduledDurationMs: 10_000,
      refreshInvalidationCount: 100,
    });
    expect(createR100Workload()).toMatchObject({
      id: "R100",
      subscriptionCount: 100,
      simultaneousContinuityLosses: 100,
      multiDocumentCount: 16,
    });
  });

  it("calculates p50 and p95 from finite nonnegative measured samples", () => {
    const summary = summarizeSamples(Array.from({ length: 30 }, (_, index) => index + 1));
    expect(summary).toMatchObject({ sampleCount: 30, p50Ms: 15, p95Ms: 29 });
    expect(() => summarizeSamples([])).toThrow(/sample_set_empty/u);
    expect(() => summarizeSamples([1, Number.NaN])).toThrow(/sample_invalid/u);
  });

  it("distinguishes noise, observations, candidates, and three-run regressions", () => {
    expect(classifyP95Regression(100, [104])).toEqual({ deltaPercent: 4, state: "noise" });
    expect(classifyP95Regression(100, [110])).toEqual({ deltaPercent: 10, state: "observe" });
    expect(classifyP95Regression(100, [116])).toEqual({ deltaPercent: 16, state: "candidate" });
    expect(classifyP95Regression(100, [116, 117, 118])).toEqual({
      deltaPercent: 17,
      state: "confirmed",
    });
  });

  it("requires every recorded B1 identity field instead of inferring qualification", () => {
    const environment = {
      platform: "linux",
      architecture: "x64",
      kernel: "6.12.0",
      cpuModel: "Pinned B1 CPU",
      logicalCpuCount: 8,
      memoryBytes: 16 * 1024 ** 3,
      cpuGovernor: "performance",
      dedicated: true,
      loopback: true,
      playwrightVersion: "1.62.1",
      browserName: "chromium" as const,
      browserVersion: "140.0.7339.16",
      browserRevision: "1194",
      viewport: { width: 1280, height: 720 },
      cpuThrottleRate: 4,
      extensions: false,
      warmHttpCache: true,
    };
    expect(classifyBenchmarkEnvironment(environment)).toBe("b1");
    expect(classifyBenchmarkEnvironment({ ...environment, dedicated: false })).toBe("exploratory");
    expect(classifyBenchmarkEnvironment({ ...environment, logicalCpuCount: 16 })).toBe(
      "exploratory",
    );
  });

  it("validates the checked baseline and fails release requests closed without B1 proof", async () => {
    const source = await readFile(
      new URL("../benchmarks/baselines/browser-budget-v1.json", import.meta.url),
      "utf8",
    );
    const baseline = validateBrowserBudgetResult(JSON.parse(source) as unknown);
    expect(baseline.classification).toBe("exploratory");
    expect(baseline.methodology.warmupSamples).toBeGreaterThan(0);
    expect(baseline.methodology.measuredSamples).toBeGreaterThanOrEqual(30);
    expect(baseline.methodology.idleDurationMs).toBe(30_000);
    expect(baseline.methodology.retainedMemory).toBe("d100-minus-control-minus-fixed-runtime-v1");
    expect(baseline.methodology.morphMeasurement).toBe("bundled-production-morph-port-v1");
    expect(baseline.methodology.asyncMeasurement).toBe("hashed-production-async-esm-v1");
    expect(baseline.methodology.morphDeadlineMs).toBe(10_000);
    expect(baseline.artifact.brotliBytes).toBeGreaterThan(0);
    expect(baseline.asyncArtifact).toMatchObject({ file: "suprnova-live.async.esm.js" });
    expect(baseline.workloads.E100).toMatchObject({
      artifactSha256: baseline.asyncArtifact.sha256,
      subscriptionCount: 100,
      presentationEventCount: 1_000,
      eventEnvelopeBytes: 1_024,
      refreshInvalidationCount: 100,
      physicalConnectionCount: 1,
      handshakeCount: 1,
      currentSubscriptionCount: 100,
    });
    expect(baseline.workloads.R100).toMatchObject({
      artifactSha256: baseline.asyncArtifact.sha256,
      subscriptionCount: 100,
      simultaneousContinuityLosses: 100,
      documentReconnectHandshakes: 1,
      recoveredSubscriptionCount: 100,
      currentSubscriptionCount: 100,
      starvedSubscriptionCount: 0,
      multiDocument: {
        documentCount: 16,
        completedHandshakes: 16,
        maximumConcurrentHandshakes: 8,
      },
    });
    expect(evaluateBrowserBudget(baseline, baseline, { release: false }).status).toBe("pass");
    expect(
      evaluateBrowserBudget(
        {
          ...baseline,
          artifact: {
            ...baseline.artifact,
            brotliBytes: baseline.artifact.brotliBytes + 4 * 1024,
          },
        },
        undefined,
        { release: false },
      ).status,
    ).toBe("pass");
    expect(evaluateBrowserBudget(baseline, baseline, { release: true })).toMatchObject({
      status: "unqualified",
      codes: ["b1_required"],
    });

    const b1Environment = {
      ...baseline.environment,
      dedicated: true,
      logicalCpuCount: 8,
      memoryBytes: 16 * 1024 ** 3,
      cpuGovernor: "performance",
    };
    const samples = (value: number) => summarizeSamples(Array.from({ length: 90 }, () => value));
    const b1Evidence = (recordedAt: string, dispatchP95Ms: number) =>
      validateBrowserBudgetResult({
        ...baseline,
        classification: "b1",
        recordedAt,
        environment: b1Environment,
        methodology: { ...baseline.methodology, independentRuns: 3 },
        workloads: {
          D100: {
            ...baseline.workloads.D100,
            connect: samples(1),
            idleMainThreadMs: 0,
            retainedBytesPerIsland: 0,
          },
          M1K: { ...baseline.workloads.M1K, morph: samples(1) },
          M5K: { ...baseline.workloads.M5K, morph: samples(1) },
          E100: {
            ...baseline.workloads.E100,
            dispatchEffect: samples(dispatchP95Ms),
            retainedBytesPerSubscription: 1,
          },
          R100: { ...baseline.workloads.R100, recovery: samples(1), retainedBytesPerIsland: 1 },
        },
        independentP95Ms: {
          d100Connect: [1, 1, 1],
          m1kMorph: [1, 1, 1],
          m5kMorph: [1, 1, 1],
          e100DispatchEffect: [dispatchP95Ms, dispatchP95Ms, dispatchP95Ms],
          r100Recovery: [1, 1, 1],
        },
      });
    const b1Candidate = b1Evidence("2026-08-27T00:00:00.000Z", 1.2);
    expect(evaluateBrowserBudget(b1Candidate, baseline, { release: true })).toMatchObject({
      status: "unqualified",
      codes: ["b1_baseline_required"],
    });
    const foreignB1 = validateBrowserBudgetResult({
      ...b1Evidence("2026-08-25T00:00:00.000Z", 1),
      environment: { ...b1Environment, cpuModel: "Different reviewed B1 CPU" },
    });
    expect(evaluateBrowserBudget(b1Candidate, foreignB1, { release: true })).toMatchObject({
      status: "unqualified",
      codes: ["b1_baseline_environment_mismatch"],
    });
    const matchingB1 = b1Evidence("2026-08-25T00:00:00.000Z", 1);
    const matchingB1Evaluation = evaluateBrowserBudget(b1Candidate, matchingB1, { release: true });
    expect(matchingB1Evaluation.status).toBe("failed");
    expect(matchingB1Evaluation.codes).toContain("e100DispatchEffect_regression_confirmed");
    expect(() =>
      validateBrowserBudgetResult({
        ...baseline,
        workloads: {
          ...baseline.workloads,
          E100: { ...baseline.workloads.E100, artifactSha256: "0".repeat(64) },
        },
      }),
    ).toThrow(/async_workload_artifact_mismatch/u);

    const tooFew = {
      ...baseline,
      classification: "b1",
      environment: {
        ...baseline.environment,
        dedicated: true,
        logicalCpuCount: 8,
        memoryBytes: 16 * 1024 ** 3,
        cpuGovernor: "performance",
      },
      workloads: {
        ...baseline.workloads,
        D100: {
          ...baseline.workloads.D100,
          connect: summarizeSamples(baseline.workloads.D100.connect.samplesMs.slice(0, 29)),
        },
      },
    };
    expect(() => validateBrowserBudgetResult(tooFew)).toThrow(/sample_count_b1/u);

    const mismatchedAsyncSampleCount = {
      ...baseline,
      workloads: {
        ...baseline.workloads,
        E100: {
          ...baseline.workloads.E100,
          dispatchEffect: summarizeSamples([
            ...baseline.workloads.E100.dispatchEffect.samplesMs,
            baseline.workloads.E100.dispatchEffect.p95Ms,
          ]),
        },
      },
    };
    expect(() => validateBrowserBudgetResult(mismatchedAsyncSampleCount)).toThrow(
      /sample_count_methodology/u,
    );

    const noisyRecoveryTiming = {
      ...baseline,
      workloads: {
        ...baseline.workloads,
        R100: {
          ...baseline.workloads.R100,
          recovery: summarizeSamples(
            baseline.workloads.R100.recovery.samplesMs.map((sample) => sample * 2),
          ),
        },
      },
      independentP95Ms: {
        ...baseline.independentP95Ms,
        r100Recovery: baseline.independentP95Ms.r100Recovery.map((sample) => sample * 2),
      },
    };
    expect(evaluateBrowserBudget(noisyRecoveryTiming, baseline, { release: false })).toMatchObject({
      status: "pass",
      codes: [],
    });

    const invalidAsyncResourceEvidence = {
      ...baseline,
      classification: "b1",
      environment: {
        ...baseline.environment,
        dedicated: true,
        logicalCpuCount: 8,
        memoryBytes: 16 * 1024 ** 3,
        cpuGovernor: "performance",
      },
      workloads: {
        ...baseline.workloads,
        E100: {
          ...baseline.workloads.E100,
          physicalConnectionCount: 2,
          queuedEventPeak: 65,
          queuedBytePeak: 256 * 1024 + 1,
          maximumQueuedRefreshesPerIsland: 2,
          maximumInFlightRefreshesPerIsland: 2,
        },
        R100: {
          ...baseline.workloads.R100,
          documentReconnectHandshakes: 2,
          pollingMaximumSameTick: 100,
        },
      },
    };
    const invalidAsyncEvaluation = evaluateBrowserBudget(
      validateBrowserBudgetResult(invalidAsyncResourceEvidence),
      matchingB1,
      {
        release: true,
      },
    );
    expect(invalidAsyncEvaluation.status).toBe("failed");
    expect(invalidAsyncEvaluation.codes).toContain("e100_physical_connection_count");
    expect(invalidAsyncEvaluation.codes).toContain("e100_queued_events_exceeded");
    expect(invalidAsyncEvaluation.codes).toContain("e100_queued_bytes_exceeded");
    expect(invalidAsyncEvaluation.codes).toContain("e100_refresh_queue_exceeded");
    expect(invalidAsyncEvaluation.codes).toContain("e100_refresh_in_flight_exceeded");
    expect(invalidAsyncEvaluation.codes).toContain("r100_document_reconnect_handshakes");
    expect(invalidAsyncEvaluation.codes).toContain("r100_polling_synchronized_burst");
  });
});
