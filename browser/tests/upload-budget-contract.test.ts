import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  U4_16,
  evaluateUploadBudget,
  regressionAtLeast15Percent,
  summarizeUploadSamples,
  validateUploadBudgetBaseline,
  validateUploadBudgetEvidence,
} from "../benchmarks/upload-schema.js";
import { estimateUploadManagerOwnedBytes } from "../benchmarks/upload-accounting.js";

const SHA256 = "a".repeat(64);
const HANDLES = Array.from(
  { length: U4_16.activeTransfers },
  (_, index) => `018f47c1-2af0-7cc4-a001-${String(index + 1).padStart(12, "0")}`,
);

function browserChunkDistribution(): Record<string, unknown>[] {
  return HANDLES.map((handle, index) => ({
    currentBytes: index === 0 ? 2 * U4_16.chunkBytes : 0,
    currentManagerBuffers: index === 0 ? 1 : 0,
    currentTotalBuffers: index === 0 ? 2 : 0,
    currentTransportBuffers: index === 0 ? 1 : 0,
    handle,
    managerHighWater: 1,
    managerHighWaterBytes: U4_16.chunkBytes,
    totalHighWater: 2,
    totalHighWaterBytes: 2 * U4_16.chunkBytes,
    transportHighWater: 1,
    transportHighWaterBytes: U4_16.chunkBytes,
  }));
}

function serverChunkDistribution(): Record<string, unknown>[] {
  return HANDLES.map((handle) => ({
    bodyHighWater: 1,
    currentBodyBuffers: 1,
    currentBytes: U4_16.chunkBytes + 64 * 1024,
    currentProviderBuffers: 1,
    currentTotalBuffers: 2,
    handle,
    providerHighWater: 1,
    totalHighWater: 2,
    totalHighWaterBytes: U4_16.chunkBytes + 64 * 1024,
  }));
}

function evidence(): Record<string, unknown> {
  const candidate: Record<string, unknown> = {
    schemaVersion: 1,
    workload: "U4/16",
    artifact: {
      brotliBytes: 10_000,
      file: "suprnova-live.uploads.esm.js",
      role: "uploads-esm",
      sha256: SHA256,
    },
    browser: {
      bounds: {
        maxChunksPerActiveTransfer: 2,
        maxManagerOwnedBytes: 256 * 1024,
        maxProgressP95Milliseconds: 16,
      },
      environment: {
        architecture: "x86_64",
        browser: "chromium",
        browserRevision: "fixture",
        classification: "unqualified",
        cpuModel: "fixture",
        cpuThrottleRate: 4,
        dedicatedVcpusAttested: false,
        extensions: false,
        kernel: "fixture",
        memoryBytes: 16 * 1024 * 1024 * 1024,
        operatingSystem: "linux",
        playwrightVersion: "1.62.1",
        profile: "B1",
        qualificationRequirementsMet: false,
        selectedCpuCount: 8,
        viewport: { height: 720, width: 1280 },
        warmHttpCache: true,
      },
      measurements: {
        activeTransfers: 4,
        chunkBuffersByTransfer: browserChunkDistribution(),
        liveChunkBuffers: 2,
        managerChunkBuffers: 1,
        managerOwnedBytes: 10_432,
        managerOwnedCategories: {
          activeLeases: 4,
          bindings: 1,
          cleanupObligations: 0,
          entries: 4,
          generationFields: 1,
          observers: 1,
          ownedResources: 4,
          pendingChunkBuffers: 1,
          pendingChunkBytes: 256 * 1024,
          queuedBytes: 0,
          queuedItems: 0,
          retainedStringCodeUnits: 512,
          waitingPermits: 0,
        },
        maxActiveManagerTransfers: 4,
        maxChunksPerTransfer: 2,
        maxQueueDepth: 4,
        maxSimultaneousTransportOperations: 4,
        maxSimultaneousTransportTransfers: 4,
        progressP50Milliseconds: 1,
        progressP95Milliseconds: 2,
        retainedBytes: 2 * 256 * 1024 + 10_432,
        slicedBytes: 4 * 16 * 1024 * 1024,
        slices: 4 * 64,
        transportChunkBuffers: 1,
      },
      methodology: {
        independentRuns: 1,
        measuredSamples: 30,
        regressionReference: "median_run_p95_v1",
        warmupIterations: 5,
      },
      workload: {
        activeTransfers: 4,
        chunkBytes: 256 * 1024,
        fileBytes: 16 * 1024 * 1024,
        files: 4,
      },
    },
    recordedAt: "2026-08-29T00:00:00.000Z",
    server: {
      bounds: {
        maxChunksPerActiveTransfer: 2,
        maxControlP95Microseconds: 2_000,
        maxManagerOwnedBytes: 512 * 1024,
      },
      environment: {
        architecture: "x86_64",
        classification: "unqualified",
        cpuGovernor: "powersave",
        cpuModel: "fixture",
        database: "not_used",
        dedicatedVcpusAttested: false,
        kernel: "fixture",
        loopbackProviders: true,
        memoryBytes: 16 * 1024 * 1024 * 1024,
        operatingSystem: "linux",
        profile: "S1",
        qualificationRequirementsMet: false,
        rustc: "fixture",
        selectedCpuCount: 8,
        warmFilesystemCache: true,
      },
      measurements: {
        chunkBuffersByTransfer: serverChunkDistribution(),
        completedBytes: 4 * 16 * 1024 * 1024,
        completedChunks: 4 * 64,
        completedTransfers: HANDLES.map((handle) => ({
          acceptedBytes: 16 * 1024 * 1024,
          acceptedChunks: 64,
          duplicateDisposition: "existing_outcome",
          finalRevision: 71,
          handle,
          providerCheckpointChunks: 64,
          providerCommittedBytes: 16 * 1024 * 1024,
        })),
        excludedCalls: {
          applicationValidation: 0,
          bodyIo: 0,
          provider: 0,
          scanner: 0,
        },
        liveChunkBuffers: 8,
        managerOwnedBytes: 52_624,
        managerOwnedCategories: {
          activeServicePermits: 4,
          providerAcceptedChunkRecords: 252,
          providerActiveChunks: 4,
          providerActiveDescriptors: 4,
          providerActiveOperations: 4,
          providerControlRecords: 1,
          providerOwnedTransfers: 4,
          retainedHandleBytes: 144,
          serviceControlRecords: 1,
        },
        maxChunksPerTransfer: 2,
        maxConcurrentTransfers: 4,
        maxQueueDepth: 4,
        p50Microseconds: 50,
        p95Microseconds: 100,
        retainedBytes: 4 * (256 * 1024 + 64 * 1024) + 52_624,
      },
      methodology: {
        measuredSamples: 40,
        warmupIterations: 50,
      },
      workload: {
        activeTransfers: 4,
        chunkBytes: 256 * 1024,
        fileBytes: 16 * 1024 * 1024,
        files: 4,
      },
    },
  };
  const browser = candidate["browser"] as Record<string, unknown>;
  const samples = Array.from({ length: 30 }, (_, index) => (index < 15 ? 1 : 2));
  browser["runs"] = [
    {
      artifactSha256: SHA256,
      environment: structuredClone(browser["environment"]),
      measurements: {
        ...(structuredClone(browser["measurements"]) as Record<string, unknown>),
        progressDurationsMilliseconds: samples,
      },
      methodology: { measuredSamples: 30, warmupIterations: 5 },
      runIndex: 1,
      workload: structuredClone(browser["workload"]),
    },
  ];
  return candidate;
}

function nestedMutation(
  mutate: (candidate: Record<string, unknown>) => void,
): Record<string, unknown> {
  const candidate = structuredClone(evidence());
  mutate(candidate);
  return candidate;
}

function qualifyThreeRuns(value: Record<string, unknown>): void {
  const browser = value["browser"] as {
    environment: Record<string, unknown>;
    methodology: { independentRuns: number; measuredSamples: number };
    runs: Record<string, unknown>[];
  };
  const server = value["server"] as { environment: Record<string, unknown> };
  browser.environment = {
    ...browser.environment,
    classification: "qualified",
    dedicatedVcpusAttested: true,
    qualificationRequirementsMet: true,
  };
  server.environment = {
    ...server.environment,
    classification: "qualified",
    cpuGovernor: "performance",
    dedicatedVcpusAttested: true,
    qualificationRequirementsMet: true,
  };
  const first = browser.runs[0];
  if (first === undefined) throw new Error("fixture_run_missing");
  browser.runs = [1, 2, 3].map((runIndex) => ({
    ...structuredClone(first),
    environment: structuredClone(browser.environment),
    runIndex,
  }));
  browser.methodology.independentRuns = 3;
  browser.methodology.measuredSamples = 90;
}

function regressRuns(value: Record<string, unknown>, count: number, p95: number): void {
  const runs = (
    value["browser"] as {
      runs: {
        measurements: {
          progressDurationsMilliseconds: number[];
          progressP95Milliseconds: number;
        };
      }[];
    }
  ).runs;
  for (const run of runs.slice(0, count)) {
    run.measurements.progressDurationsMilliseconds.splice(-2, 2, p95, p95);
    run.measurements.progressP95Milliseconds = p95;
  }
  const allSamples = runs.flatMap(({ measurements }) => measurements.progressDurationsMilliseconds);
  const browser = value["browser"] as {
    measurements: { progressP50Milliseconds: number; progressP95Milliseconds: number };
  };
  const summary = summarizeUploadSamples(allSamples);
  browser.measurements.progressP50Milliseconds = summary.p50;
  browser.measurements.progressP95Milliseconds = summary.p95;
}

describe("U4/16 evidence schema", () => {
  it("validates the checked exploratory envelope without inventing a qualified baseline", async () => {
    const checked: unknown = JSON.parse(
      await readFile(
        new URL("../benchmarks/baselines/upload-budget-v1.json", import.meta.url),
        "utf8",
      ),
    ) as unknown;
    const baseline = validateUploadBudgetBaseline(checked);
    expect(baseline.exploratoryReference.workload).toBe("U4/16");
    expect(baseline.qualifiedBaseline).toBeNull();
  });

  it("locks the exact workload, hard bounds, sample counts, and artifact binding", () => {
    const validated = validateUploadBudgetEvidence(evidence());

    expect(validated.workload).toBe("U4/16");
    expect(validated.browser.workload).toEqual(U4_16);
    expect(validated.server.workload).toEqual(U4_16);
    expect(validated.artifact.sha256).toBe(SHA256);
    expect(validated.browser.methodology.measuredSamples).toBeGreaterThanOrEqual(30);
    expect(validated.server.methodology.measuredSamples).toBeGreaterThanOrEqual(30);
  });

  it.each([
    ["workload", (value: Record<string, unknown>) => (value["workload"] = "U3/16")],
    [
      "files",
      (value: Record<string, unknown>) => {
        (value["browser"] as { workload: { files: number } }).workload.files = 3;
      },
    ],
    [
      "file bytes",
      (value: Record<string, unknown>) => {
        (value["browser"] as { workload: { fileBytes: number } }).workload.fileBytes -= 1;
      },
    ],
    [
      "chunk bytes",
      (value: Record<string, unknown>) => {
        (value["server"] as { workload: { chunkBytes: number } }).workload.chunkBytes /= 2;
      },
    ],
    [
      "active transfers",
      (value: Record<string, unknown>) => {
        (value["browser"] as { workload: { activeTransfers: number } }).workload.activeTransfers =
          3;
      },
    ],
    [
      "samples",
      (value: Record<string, unknown>) => {
        (
          value["browser"] as { methodology: { measuredSamples: number } }
        ).methodology.measuredSamples = 29;
      },
    ],
    [
      "omitted samples",
      (value: Record<string, unknown>) => {
        delete (value["browser"] as { methodology: { measuredSamples?: number } }).methodology
          .measuredSamples;
      },
    ],
    [
      "warmup",
      (value: Record<string, unknown>) => {
        (
          value["server"] as { methodology: { warmupIterations: number } }
        ).methodology.warmupIterations = 0;
      },
    ],
    [
      "artifact hash",
      (value: Record<string, unknown>) => {
        delete (value["artifact"] as { sha256?: string }).sha256;
      },
    ],
    [
      "classification",
      (value: Record<string, unknown>) => {
        (
          value["browser"] as { environment: { classification: string } }
        ).environment.classification = "maybe";
      },
    ],
    [
      "excluded counter",
      (value: Record<string, unknown>) => {
        (
          value["server"] as {
            measurements: { excludedCalls: { provider: number } };
          }
        ).measurements.excludedCalls.provider = 1;
      },
    ],
    [
      "underestimated retained bytes",
      (value: Record<string, unknown>) => {
        (
          value["browser"] as { measurements: { retainedBytes: number } }
        ).measurements.retainedBytes = 1;
      },
    ],
    [
      "underestimated chunk buffers",
      (value: Record<string, unknown>) => {
        const browser = value["browser"] as {
          measurements: { liveChunkBuffers: number };
          runs: { measurements: { liveChunkBuffers: number } }[];
        };
        browser.measurements.liveChunkBuffers = 1;
        const run = browser.runs[0];
        if (run === undefined) throw new Error("fixture_run_missing");
        run.measurements.liveChunkBuffers = 1;
      },
    ],
    [
      "unknown field",
      (value: Record<string, unknown>) => {
        (value["browser"] as Record<string, unknown>)["surprise"] = true;
      },
    ],
  ])("rejects the %s mutation", (_name, mutate) => {
    expect(() => validateUploadBudgetEvidence(nestedMutation(mutate))).toThrow(
      "upload_budget_evidence_invalid",
    );
  });

  it("uses nearest-rank percentiles over every measured sample", () => {
    const summary = summarizeUploadSamples([
      9, 1, 8, 2, 7, 3, 6, 4, 5, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    ]);
    expect(summary).toEqual({ p50: 10, p95: 19 });
  });

  it("treats just-below, exact, and above-threshold ratios precisely", () => {
    expect(regressionAtLeast15Percent(2.299_999_999, 2)).toBe(false);
    expect(regressionAtLeast15Percent(2.3, 2)).toBe(true);
    expect(regressionAtLeast15Percent(2.300_000_001, 2)).toBe(true);
  });

  it.each(["artifact", "environment", "samples"] as const)(
    "fails closed when one independent run has mismatched %s evidence",
    (mutation) => {
      const value = evidence();
      qualifyThreeRuns(value);
      const runs = (value["browser"] as { runs: Record<string, unknown>[] }).runs;
      const selected = runs[1];
      if (selected === undefined) throw new Error("fixture_run_missing");
      if (mutation === "artifact") selected["artifactSha256"] = "b".repeat(64);
      else if (mutation === "environment") {
        (selected["environment"] as Record<string, unknown>)["browserRevision"] = "mismatch";
      } else {
        (selected["methodology"] as Record<string, unknown>)["measuredSamples"] = 29;
      }
      expect(() => validateUploadBudgetEvidence(value)).toThrow("upload_budget_evidence_invalid");
    },
  );

  it("fails hard bounds, artifact mismatch, regressions, and false qualification independently", () => {
    const baseline = validateUploadBudgetEvidence(evidence());
    const overCap = nestedMutation((value) => {
      const browser = value["browser"] as {
        measurements: { progressP50Milliseconds: number; progressP95Milliseconds: number };
        runs: {
          measurements: {
            progressDurationsMilliseconds: number[];
            progressP50Milliseconds: number;
            progressP95Milliseconds: number;
          };
        }[];
      };
      browser.measurements.progressP50Milliseconds = 17;
      browser.measurements.progressP95Milliseconds = 17;
      for (const run of browser.runs) {
        run.measurements.progressDurationsMilliseconds.fill(17);
        run.measurements.progressP50Milliseconds = 17;
        run.measurements.progressP95Milliseconds = 17;
      }
    });
    const mismatched = nestedMutation((value) => {
      (value["artifact"] as { sha256: string }).sha256 = "b".repeat(64);
      for (const run of (value["browser"] as { runs: { artifactSha256: string }[] }).runs) {
        run.artifactSha256 = "b".repeat(64);
      }
    });
    const regressionBaselineValue = evidence();
    qualifyThreeRuns(regressionBaselineValue);
    const regressed = structuredClone(regressionBaselineValue);
    regressRuns(regressed, 3, 2.3);
    const wrongEnvironment = nestedMutation((value) => {
      const environment = (value["browser"] as { environment: { browserRevision: string } })
        .environment;
      environment.browserRevision = "different";
      for (const run of (
        value["browser"] as { runs: { environment: { browserRevision: string } }[] }
      ).runs) {
        run.environment.browserRevision = "different";
      }
    });
    const falselyQualified = nestedMutation((value) => {
      const environment = (value["browser"] as { environment: Record<string, unknown> })
        .environment;
      environment["classification"] = "qualified";
      environment["qualificationRequirementsMet"] = false;
    });

    expect(evaluateUploadBudget(validateUploadBudgetEvidence(overCap), baseline).issues).toContain(
      "upload_budget:browser:progress_p95_hard_cap",
    );
    expect(
      evaluateUploadBudget(validateUploadBudgetEvidence(mismatched), baseline, {
        artifactSha256: SHA256,
      }).issues,
    ).toContain("upload_budget:artifact_mismatch");
    expect(
      evaluateUploadBudget(
        validateUploadBudgetEvidence(regressed),
        validateUploadBudgetEvidence(regressionBaselineValue),
      ).issues,
    ).toContain("upload_budget:browser:progress_p95_regression");
    expect(
      evaluateUploadBudget(validateUploadBudgetEvidence(wrongEnvironment), baseline).issues,
    ).toContain("upload_budget:baseline_environment_mismatch");
    expect(() => validateUploadBudgetEvidence(falselyQualified)).toThrow(
      "upload_budget_evidence_invalid",
    );
  });

  it("turns increased observed manager metadata into a hard-cap failure", () => {
    const mutated = nestedMutation((value) => {
      const browser = value["browser"] as {
        measurements: {
          liveChunkBuffers: number;
          managerOwnedBytes: number;
          managerOwnedCategories: Parameters<typeof estimateUploadManagerOwnedBytes>[0];
          retainedBytes: number;
        };
        runs: {
          measurements: {
            liveChunkBuffers: number;
            managerOwnedBytes: number;
            managerOwnedCategories: Parameters<typeof estimateUploadManagerOwnedBytes>[0];
            retainedBytes: number;
          };
        }[];
      };
      for (const measurements of [
        browser.measurements,
        ...browser.runs.map(({ measurements }) => measurements),
      ]) {
        measurements.managerOwnedCategories = {
          ...measurements.managerOwnedCategories,
          queuedItems: 1_000,
        };
        measurements.managerOwnedBytes = estimateUploadManagerOwnedBytes(
          measurements.managerOwnedCategories,
        );
        measurements.retainedBytes =
          measurements.liveChunkBuffers * U4_16.chunkBytes + measurements.managerOwnedBytes;
      }
    });
    const evaluation = evaluateUploadBudget(validateUploadBudgetEvidence(mutated), null);
    expect(evaluation.issues).toContain("upload_budget:browser:manager_bytes_hard_cap");
  });

  it("fails a skewed three-buffer browser transfer even when document totals remain acceptable", () => {
    const mutated = nestedMutation((value) => {
      const browser = value["browser"] as {
        measurements: {
          chunkBuffersByTransfer: {
            currentBytes: number;
            currentManagerBuffers: number;
            currentTotalBuffers: number;
            managerHighWater: number;
            managerHighWaterBytes: number;
            totalHighWater: number;
            totalHighWaterBytes: number;
            transportHighWater: number;
          }[];
          liveChunkBuffers: number;
          managerChunkBuffers: number;
          managerOwnedBytes: number;
          maxChunksPerTransfer: number;
          retainedBytes: number;
        };
        runs: {
          measurements: {
            chunkBuffersByTransfer: {
              currentBytes: number;
              currentManagerBuffers: number;
              currentTotalBuffers: number;
              managerHighWater: number;
              managerHighWaterBytes: number;
              totalHighWater: number;
              totalHighWaterBytes: number;
              transportHighWater: number;
            }[];
            liveChunkBuffers: number;
            managerChunkBuffers: number;
            managerOwnedBytes: number;
            maxChunksPerTransfer: number;
            retainedBytes: number;
          };
        }[];
      };
      for (const measurements of [
        browser.measurements,
        ...browser.runs.map(({ measurements }) => measurements),
      ]) {
        const first = measurements.chunkBuffersByTransfer[0];
        if (first === undefined) throw new Error("fixture_transfer_missing");
        first.currentBytes = 3 * U4_16.chunkBytes;
        first.currentManagerBuffers = 2;
        first.currentTotalBuffers = 3;
        first.managerHighWater = 2;
        first.managerHighWaterBytes = 2 * U4_16.chunkBytes;
        first.totalHighWater = 3;
        first.totalHighWaterBytes = 3 * U4_16.chunkBytes;
        measurements.liveChunkBuffers = 3;
        measurements.managerChunkBuffers = 2;
        measurements.maxChunksPerTransfer = 3;
        measurements.retainedBytes = 3 * U4_16.chunkBytes + measurements.managerOwnedBytes;
      }
    });
    const evaluation = evaluateUploadBudget(validateUploadBudgetEvidence(mutated), null);
    expect(evaluation.issues).toContain("upload_budget:browser:chunks_per_transfer_hard_cap");
  });

  it("fails a skewed three-buffer server transfer even when the average remains two", () => {
    const mutated = nestedMutation((value) => {
      const measurements = (
        value["server"] as {
          measurements: {
            chunkBuffersByTransfer: {
              bodyHighWater: number;
              currentBytes: number;
              currentProviderBuffers: number;
              currentTotalBuffers: number;
              providerHighWater: number;
              totalHighWater: number;
              totalHighWaterBytes: number;
            }[];
            maxChunksPerTransfer: number;
          };
        }
      ).measurements;
      const first = measurements.chunkBuffersByTransfer[0];
      const second = measurements.chunkBuffersByTransfer[1];
      if (first === undefined || second === undefined) throw new Error("fixture_transfer_missing");
      first.providerHighWater = 2;
      first.totalHighWater = 3;
      first.currentProviderBuffers = 2;
      first.currentTotalBuffers = 3;
      first.currentBytes += 64 * 1024;
      first.totalHighWaterBytes += 64 * 1024;
      second.currentProviderBuffers = 0;
      second.currentTotalBuffers = 1;
      second.currentBytes -= 64 * 1024;
      measurements.maxChunksPerTransfer = 3;
    });
    const evaluation = evaluateUploadBudget(validateUploadBudgetEvidence(mutated), null);
    expect(evaluation.issues).toContain("upload_budget:server:chunks_per_transfer_hard_cap");
  });

  it("requires both environments and three independent browser runs for release qualification", () => {
    const unqualified = validateUploadBudgetEvidence(evidence());
    expect(evaluateUploadBudget(unqualified, unqualified, { release: true }).classification).toBe(
      "unqualified",
    );
    expect(evaluateUploadBudget(unqualified, unqualified, { release: true }).issues).toContain(
      "upload_budget:release_environment_unqualified",
    );
    expect(evaluateUploadBudget(unqualified, null, { release: true }).issues).toContain(
      "upload_budget:qualified_baseline_missing",
    );
  });

  it.each([
    [1, false],
    [2, false],
    [3, true],
  ])(
    "reports %i of three exact regressions and blocks only a repeated result",
    (regressedRunCount, confirmed) => {
      const baselineValue = evidence();
      qualifyThreeRuns(baselineValue);
      const candidateValue = structuredClone(baselineValue);
      regressRuns(candidateValue, regressedRunCount, 2.3);

      const evaluation = evaluateUploadBudget(
        validateUploadBudgetEvidence(candidateValue),
        validateUploadBudgetEvidence(baselineValue),
      );
      expect(evaluation.observations).toContain(
        `upload_budget:browser:progress_p95_regression_${String(regressedRunCount)}_of_3`,
      );
      expect(evaluation.issues.includes("upload_budget:browser:progress_p95_regression")).toBe(
        confirmed,
      );
    },
  );

  it("does not report just-below-threshold run noise as a regression", () => {
    const baselineValue = evidence();
    qualifyThreeRuns(baselineValue);
    const candidateValue = structuredClone(baselineValue);
    regressRuns(candidateValue, 3, 2.299_8);
    const evaluation = evaluateUploadBudget(
      validateUploadBudgetEvidence(candidateValue),
      validateUploadBudgetEvidence(baselineValue),
    );
    expect(evaluation.issues).not.toContain("upload_budget:browser:progress_p95_regression");
    expect(evaluation.observations).toEqual([]);
  });

  it("compares every candidate run to one permutation-invariant baseline median", () => {
    const baselineValue = evidence();
    qualifyThreeRuns(baselineValue);
    const baselineRuns = (baselineValue["browser"] as { runs: Record<string, unknown>[] }).runs;
    const baselineP95 = [1.5, 2, 2.5];
    baselineRuns.forEach((run, index) => {
      const measurements = run["measurements"] as {
        progressDurationsMilliseconds: number[];
        progressP50Milliseconds: number;
        progressP95Milliseconds: number;
      };
      const value = baselineP95[index] ?? 2;
      measurements.progressDurationsMilliseconds.fill(value);
      measurements.progressP50Milliseconds = value;
      measurements.progressP95Milliseconds = value;
    });
    const baselineBrowser = baselineValue["browser"] as {
      measurements: { progressP50Milliseconds: number; progressP95Milliseconds: number };
    };
    const baselineSummary = summarizeUploadSamples(
      baselineRuns.flatMap(
        (run) =>
          (run["measurements"] as { progressDurationsMilliseconds: number[] })
            .progressDurationsMilliseconds,
      ),
    );
    baselineBrowser.measurements.progressP50Milliseconds = baselineSummary.p50;
    baselineBrowser.measurements.progressP95Milliseconds = baselineSummary.p95;

    const candidateValue = structuredClone(baselineValue);
    const candidateRuns = (candidateValue["browser"] as { runs: Record<string, unknown>[] }).runs;
    for (const run of candidateRuns) {
      const measurements = run["measurements"] as {
        progressDurationsMilliseconds: number[];
        progressP50Milliseconds: number;
        progressP95Milliseconds: number;
      };
      measurements.progressDurationsMilliseconds.fill(2.3);
      measurements.progressP50Milliseconds = 2.3;
      measurements.progressP95Milliseconds = 2.3;
    }
    const candidateBrowser = candidateValue["browser"] as {
      measurements: { progressP50Milliseconds: number; progressP95Milliseconds: number };
    };
    candidateBrowser.measurements.progressP50Milliseconds = 2.3;
    candidateBrowser.measurements.progressP95Milliseconds = 2.3;
    const permutations = <T>(values: readonly T[]): T[][] =>
      values.length === 0
        ? [[]]
        : values.flatMap((value, index) =>
            permutations(values.filter((_, candidateIndex) => candidateIndex !== index)).map(
              (tail) => [value, ...tail],
            ),
          );
    const outcomes = new Set<string>();
    for (const baselinePermutation of permutations(baselineRuns)) {
      for (const candidatePermutation of permutations(candidateRuns)) {
        const permutedBaseline = structuredClone(baselineValue);
        (permutedBaseline["browser"] as { runs: Record<string, unknown>[] }).runs =
          structuredClone(baselinePermutation);
        const permutedCandidate = structuredClone(candidateValue);
        (permutedCandidate["browser"] as { runs: Record<string, unknown>[] }).runs =
          structuredClone(candidatePermutation);
        const evaluation = evaluateUploadBudget(
          validateUploadBudgetEvidence(permutedCandidate),
          validateUploadBudgetEvidence(permutedBaseline),
        );
        outcomes.add(
          JSON.stringify({
            issues: evaluation.issues,
            observations: evaluation.observations,
          }),
        );
      }
    }
    expect(outcomes).toEqual(
      new Set([
        JSON.stringify({
          issues: ["upload_budget:browser:progress_p95_regression"],
          observations: ["upload_budget:browser:progress_p95_regression_3_of_3"],
        }),
      ]),
    );
  });

  it("rejects serialized transport even when four manager leases are active", () => {
    const serialized = nestedMutation((value) => {
      const browser = value["browser"] as {
        measurements: {
          maxSimultaneousTransportOperations: number;
          maxSimultaneousTransportTransfers: number;
        };
        runs: {
          measurements: {
            maxSimultaneousTransportOperations: number;
            maxSimultaneousTransportTransfers: number;
          };
        }[];
      };
      for (const measurements of [
        browser.measurements,
        ...browser.runs.map((run) => run.measurements),
      ]) {
        measurements.maxSimultaneousTransportOperations = 1;
        measurements.maxSimultaneousTransportTransfers = 1;
      }
    });
    expect(() => validateUploadBudgetEvidence(serialized)).toThrow(
      "upload_budget_evidence_invalid",
    );
  });

  it("applies hard latency caps to every independent browser run", () => {
    const value = evidence();
    qualifyThreeRuns(value);
    regressRuns(value, 1, 17);
    const evaluation = evaluateUploadBudget(validateUploadBudgetEvidence(value), null);
    expect(evaluation.issues).toContain("upload_budget:browser:run_1:progress_p95_hard_cap");
  });

  it("keeps exploratory evidence separate from an absent qualified regression baseline", () => {
    const exploratoryReference = validateUploadBudgetEvidence(evidence());
    const baseline = validateUploadBudgetBaseline({
      exploratoryReference,
      qualifiedBaseline: null,
      schemaVersion: 1,
      workload: "U4/16",
    });
    expect(baseline.qualifiedBaseline).toBeNull();
    expect(baseline.exploratoryReference.browser.environment.classification).toBe("unqualified");

    expect(() =>
      validateUploadBudgetBaseline({
        ...baseline,
        qualifiedBaseline: exploratoryReference,
      }),
    ).toThrow("upload_budget_baseline_invalid");
  });

  it("keeps benchmark observers outside every production entry and artifact", async () => {
    const build = await readFile(new URL("../scripts/build.mjs", import.meta.url), "utf8");
    const uploadsEntry = await readFile(
      new URL("../src/entry-uploads-esm.ts", import.meta.url),
      "utf8",
    );

    expect(build).not.toContain("upload-workloads");
    expect(uploadsEntry).not.toContain("upload-workloads");
    expect(uploadsEntry).not.toContain("uploadBudgetObserver");
  });
});
