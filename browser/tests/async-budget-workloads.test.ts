import { describe, expect, it } from "vitest";

import * as asyncBudgetWorkloads from "../benchmarks/async-budget-workloads.js";

const { ASYNC_BUDGET_DRIVER_MARKER, summarizeAsyncSamples } = asyncBudgetWorkloads;

describe("async budget evidence helpers", () => {
  it("derives retained bytes only from raw forced-GC Chromium heap samples", () => {
    expect(ASYNC_BUDGET_DRIVER_MARKER).toBe("SUPRNOVA_ASYNC_BUDGET_DRIVER_V1");
    const helper = Reflect.get(asyncBudgetWorkloads, "deriveRetainedHeapMeasurement") as
      ((value: unknown) => unknown) | undefined;
    expect(helper).toBeTypeOf("function");
    expect(
      helper?.({
        after: [
          { backingStorageSize: 7, embedderHeapUsedSize: 11, usedSize: 113 },
          { backingStorageSize: 8, embedderHeapUsedSize: 12, usedSize: 115 },
        ],
        before: [
          { backingStorageSize: 5, embedderHeapUsedSize: 10, usedSize: 100 },
          { backingStorageSize: 6, embedderHeapUsedSize: 10, usedSize: 101 },
        ],
      }),
    ).toEqual({
      after: [
        { backingStorageSize: 7, embedderHeapUsedSize: 11, usedSize: 113 },
        { backingStorageSize: 8, embedderHeapUsedSize: 12, usedSize: 115 },
      ],
      before: [
        { backingStorageSize: 5, embedderHeapUsedSize: 10, usedSize: 100 },
        { backingStorageSize: 6, embedderHeapUsedSize: 10, usedSize: 101 },
      ],
      retainedBytes: 20,
    });
    expect(() =>
      helper?.({
        after: [
          { backingStorageSize: 1, embedderHeapUsedSize: 1, usedSize: Number.MAX_SAFE_INTEGER },
        ],
        before: [
          { backingStorageSize: 1, embedderHeapUsedSize: 1, usedSize: Number.MAX_SAFE_INTEGER },
        ],
      }),
    ).toThrow("async_heap_sample_invalid");
    expect(Reflect.has(asyncBudgetWorkloads, "estimateAsyncRetainedBytes")).toBe(false);
  });

  it("computes deterministic nearest-rank p50/p95 without changing sample order", () => {
    const samples = [4, 1, 3, 2];
    expect(summarizeAsyncSamples(samples)).toEqual({
      durationsMilliseconds: samples,
      p50Milliseconds: 2,
      p95Milliseconds: 4,
      sampleCount: 4,
    });
    expect(samples).toEqual([4, 1, 3, 2]);
  });
});
