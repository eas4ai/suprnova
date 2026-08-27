import { mkdir, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

import { expect, test } from "@playwright/test";

import { runBrowserBudget } from "../benchmarks/runner.js";
import { createD100Workload, createMorphWorkload } from "../benchmarks/workloads.js";

function boundedInteger(name: string, fallback: number, maximum: number): number {
  const raw = process.env[name];
  if (raw === undefined) return fallback;
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw new Error(`invalid benchmark setting: ${name}`);
  }
  return value;
}

test("canonical browser workloads remain executable without changing engine claims", async ({
  baseURL,
  browser,
  browserName,
  context,
}) => {
  const d100 = createD100Workload();
  const m1k = createMorphWorkload("M1K");
  const m5k = createMorphWorkload("M5K");
  expect(Buffer.byteLength(d100.html, "utf8")).toBe(65_536);
  expect(m1k.changedNodeCount).toBe(100);
  expect(m5k.changedNodeCount).toBe(500);

  // Firefox and WebKit remain correctness engines. B1 is explicitly pinned Chromium/CDP evidence.
  if (browserName !== "chromium") return;
  if (baseURL === undefined) throw new Error("benchmark base URL missing");
  const recording = process.env["SUPRNOVA_BROWSER_BUDGET_RECORD"] === "1";
  test.setTimeout(recording ? 15 * 60_000 : 60_000);
  const result = await runBrowserBudget({
    browser,
    context,
    baseUrl: baseURL,
    warmupSamples: boundedInteger("SUPRNOVA_BENCHMARK_WARMUPS", 1, 100),
    measuredSamples: boundedInteger("SUPRNOVA_BENCHMARK_SAMPLES", 1, 100),
    independentRuns: boundedInteger("SUPRNOVA_BENCHMARK_RUNS", 1, 3),
    idleDurationMs: boundedInteger("SUPRNOVA_BENCHMARK_IDLE_MS", 100, 120_000),
    cpuThrottleRate: boundedInteger("SUPRNOVA_BENCHMARK_CPU_THROTTLE", 1, 16),
    dedicated: process.env["SUPRNOVA_BENCHMARK_DEDICATED"] === "1",
  });
  expect(result.workloads.D100.connect.sampleCount).toBeGreaterThan(0);
  expect(result.workloads.M1K.morph.sampleCount).toBeGreaterThan(0);
  expect(result.workloads.M5K.morph.sampleCount).toBeGreaterThan(0);
  expect(result.workloads.E100).toMatchObject({
    currentSubscriptionCount: 100,
    handshakeCount: 1,
    physicalConnectionCount: 1,
  });
  expect(result.workloads.R100).toMatchObject({
    currentSubscriptionCount: 100,
    documentReconnectHandshakes: 1,
    recoveredSubscriptionCount: 100,
    starvedSubscriptionCount: 0,
  });
  expect(result.workloads.R100.multiDocument).toMatchObject({
    completedHandshakes: 16,
    maximumConcurrentHandshakes: 8,
  });

  const output = process.env["SUPRNOVA_BROWSER_BUDGET_OUTPUT"];
  if (output !== undefined) {
    const destination = resolve(output);
    const temporary = `${destination}.${String(process.pid)}.temporary`;
    await mkdir(dirname(destination), { recursive: true });
    await writeFile(temporary, `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
    await rename(temporary, destination);
  }
});
