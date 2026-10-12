import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import { argumentsFrom, type BrowserBudgetArguments } from "../scripts/run-browser-budget.mjs";

describe("browser benchmark runner", () => {
  it("keeps the declared runner arguments identical to the runtime parser", () => {
    const parsed: BrowserBudgetArguments = argumentsFrom([
      "--baseline",
      "benchmarks/baselines/prior.json",
      "--dedicated",
      "--idle-ms",
      "30000",
      "--output",
      "benchmarks/local/current.json",
      "--release",
      "--runs",
      "3",
      "--samples",
      "30",
      "--warmups",
      "5",
    ]);

    expect(Object.keys(parsed).sort()).toEqual([
      "baseline",
      "dedicated",
      "idleMs",
      "output",
      "release",
      "runs",
      "samples",
      "warmups",
    ]);
    expect(parsed.dedicated).toBe(true);
    expect(parsed.idleMs).toBe(30_000);
    expect(parsed.release).toBe(true);
    expect(parsed.runs).toBe(3);
    expect(parsed.samples).toBe(30);
    expect(parsed.warmups).toBe(5);
  });

  it("defaults binding evidence to three independent runs", () => {
    expect(argumentsFrom([]).runs).toBe(3);
  });

  it("stays an on-demand tool that the gate never runs", async () => {
    const gate = await readFile(new URL("../../scripts/gate.sh", import.meta.url), "utf8");
    expect(gate).not.toContain("npm run budget");
  });

  it("measures async workloads through the exact built ESM artifact, never a source bundle", async () => {
    const runner = await readFile(new URL("../benchmarks/runner.ts", import.meta.url), "utf8");
    const workload = await readFile(
      new URL("../benchmarks/async-workloads.ts", import.meta.url),
      "utf8",
    );
    expect(runner).not.toContain("asyncHarnessSource");
    expect(runner).not.toContain("browser-budget-async-port.ts");
    expect(workload).toContain("await import(artifactUrl)");
    expect(workload).not.toMatch(/from\s+["']\.\.\/src\/async-updates/u);
    expect(workload).not.toContain("pollingOwner");
    expect(workload).toContain("timers.scheduledCountAfter");
    expect(workload).toContain("timers.maximumSameDueAfter");
  });

  it("refuses to overwrite the binding baseline with its own candidate output", async () => {
    const loaded = (await import("../scripts/run-browser-budget.mjs")) as {
      readonly argumentsFrom: (arguments_: readonly string[]) => unknown;
    };

    expect(() =>
      loaded.argumentsFrom([
        "--baseline",
        "benchmarks/baselines/browser-budget-v1.json",
        "--output",
        "benchmarks/baselines/browser-budget-v1.json",
      ]),
    ).toThrow("baseline_overwrite_forbidden");
  });
});
