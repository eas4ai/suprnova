import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  U4_16,
  evaluateUploadBudget,
  summarizeUploadSamples,
  validateUploadBudgetBaseline,
  validateUploadBudgetEvidence,
} from "../benchmarks/upload-schema.js";
import { estimateUploadManagerOwnedBytes } from "../benchmarks/upload-accounting.js";

const SHA256 = "a".repeat(64);

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
        maxChunksPerTransfer: 2,
        maxConcurrentTransfers: 4,
        maxQueueDepth: 4,
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
        excludedCalls: {
          applicationValidation: 0,
          bodyIo: 0,
          provider: 0,
          scanner: 0,
        },
        liveChunkBuffers: 8,
        managerOwnedBytes: 4_240,
        managerOwnedCategories: {
          activePermits: 4,
          chunkQueueEntries: 8,
          permitSlots: 4,
          queueControlRecords: 2,
          retainedHandleBytes: 144,
          transferQueueEntries: 4,
        },
        maxChunksPerTransfer: 2,
        maxConcurrentTransfers: 4,
        maxQueueDepth: 4,
        p50Microseconds: 50,
        p95Microseconds: 100,
        retainedBytes: 8 * 256 * 1024 + 4_240 + 144,
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
    const regressed = nestedMutation((value) => {
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
      browser.measurements.progressP50Milliseconds = 2.3;
      browser.measurements.progressP95Milliseconds = 2.3;
      for (const run of browser.runs) {
        run.measurements.progressDurationsMilliseconds.fill(2.3);
        run.measurements.progressP50Milliseconds = 2.3;
        run.measurements.progressP95Milliseconds = 2.3;
      }
    });
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
      evaluateUploadBudget(validateUploadBudgetEvidence(regressed), baseline).issues,
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

  it("rejects a 15-percent regression isolated to one Chromium process", () => {
    const threeRuns = (value: Record<string, unknown>): void => {
      const browser = value["browser"] as {
        methodology: { independentRuns: number; measuredSamples: number };
        runs: Record<string, unknown>[];
      };
      const first = browser.runs[0];
      if (first === undefined) throw new Error("fixture_run_missing");
      browser.runs = [1, 2, 3].map((runIndex) => ({ ...structuredClone(first), runIndex }));
      browser.methodology.independentRuns = 3;
      browser.methodology.measuredSamples = 90;
    };
    const baselineValue = evidence();
    threeRuns(baselineValue);
    const candidateValue = structuredClone(baselineValue);
    const run = (
      candidateValue["browser"] as {
        runs: {
          measurements: {
            progressDurationsMilliseconds: number[];
            progressP95Milliseconds: number;
          };
        }[];
      }
    ).runs[1];
    if (run === undefined) throw new Error("fixture_run_missing");
    run.measurements.progressDurationsMilliseconds.splice(-2, 2, 2.3, 2.3);
    run.measurements.progressP95Milliseconds = 2.3;

    const evaluation = evaluateUploadBudget(
      validateUploadBudgetEvidence(candidateValue),
      validateUploadBudgetEvidence(baselineValue),
    );
    expect(evaluation.issues).toContain("upload_budget:browser:run_2:progress_p95_regression");
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
