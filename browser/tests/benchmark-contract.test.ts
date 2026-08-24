import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import {
  classifyBenchmarkEnvironment,
  evaluateBrowserBudget,
  validateBrowserBudgetResult,
} from "../benchmarks/schema.js";
import { classifyP95Regression, summarizeSamples } from "../benchmarks/statistics.js";
import { createD100Workload, createMorphWorkload } from "../benchmarks/workloads.js";

describe("browser performance evidence contract", () => {
  it("generates the exact D100, M1K, and M5K workload shapes", () => {
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
    expect(baseline.methodology.morphDeadlineMs).toBe(10_000);
    expect(baseline.artifact.brotliBytes).toBeGreaterThan(0);
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
  });
});
