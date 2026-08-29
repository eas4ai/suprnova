import { describe, expect, it } from "vitest";

import {
  ASYNC_BUDGET_DRIVER_MARKER,
  estimateAsyncRetainedBytes,
  summarizeAsyncSamples,
} from "../benchmarks/async-budget-workloads.js";

describe("async budget evidence helpers", () => {
  it("uses a closed per-subscription accounting model instead of dividing a document total", () => {
    expect(ASYNC_BUDGET_DRIVER_MARKER).toBe("SUPRNOVA_ASYNC_BUDGET_DRIVER_V1");
    const first = estimateAsyncRetainedBytes({
      authorizationBytes: 512,
      identifierBytes: 128,
      pendingBytes: 0,
      pendingEvents: 0,
      pollTimers: 0,
      refreshSlots: 0,
      runtimeRecords: 7,
    });
    const chatty = estimateAsyncRetainedBytes({
      authorizationBytes: 512,
      identifierBytes: 128,
      pendingBytes: 4_096,
      pendingEvents: 4,
      pollTimers: 1,
      refreshSlots: 1,
      runtimeRecords: 7,
    });
    expect(first).toBe(2_176);
    expect(chatty).toBe(7_168);
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
