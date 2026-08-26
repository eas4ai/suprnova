import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

import { argumentsFrom, type BrowserBudgetArguments } from "../scripts/run-browser-budget.mjs";

type ArtifactBudgetInput = Readonly<{
  role: string;
  file: string;
  compatibleCore: string;
  brotliBytes: number;
}>;

type ArtifactBudgetEvaluation = Readonly<{
  lines: readonly string[];
  issues: readonly string[];
}>;

const EXPECTED = Object.freeze([
  ["core-esm", null],
  ["core-classic", null],
  ["stimulus-esm", 8 * 1024],
  ["stimulus-classic", 8 * 1024],
  ["uploads-esm", 20 * 1024],
  ["uploads-classic", 20 * 1024],
  ["async-esm", 16 * 1024],
  ["async-classic", 16 * 1024],
] as const);

async function evaluator(): Promise<
  (assets: readonly ArtifactBudgetInput[]) => ArtifactBudgetEvaluation
> {
  const moduleUrl = new URL("../scripts/check-budget.mjs", import.meta.url);
  const loaded = (await import(moduleUrl.href)) as {
    readonly evaluateArtifactBudgets: (
      assets: readonly ArtifactBudgetInput[],
    ) => ArtifactBudgetEvaluation;
  };
  return loaded.evaluateArtifactBudgets;
}

function validArtifacts(): ArtifactBudgetInput[] {
  return EXPECTED.map(([role, ceiling]) => ({
    role,
    file: `${role}.js`,
    compatibleCore: ">=0.1.0 <0.2.0",
    brotliBytes: ceiling ?? 128 * 1024,
  }));
}

describe("role-aware production artifact budgets", () => {
  it("reports measurement-only core roles and each optional artifact ceiling", async () => {
    const evaluate = await evaluator();
    const result = evaluate(validArtifacts());

    expect(result.issues).toEqual([]);
    expect(result.lines).toEqual(
      EXPECTED.map(
        ([role, ceiling]) =>
          `artifact_budget role=${role} bytes=${String(ceiling ?? 128 * 1024)} ceiling=${String(ceiling ?? "none")}`,
      ),
    );
  });

  it("reports duplicate, missing, incompatible, and over-budget roles together", async () => {
    const evaluate = await evaluator();
    const artifacts = validArtifacts();
    const duplicate = artifacts[0];
    if (duplicate === undefined) throw new Error("duplicate_fixture_missing");
    artifacts.push({ ...duplicate });
    artifacts.splice(
      artifacts.findIndex(({ role }) => role === "async-classic"),
      1,
    );
    const uploads = artifacts.find(({ role }) => role === "uploads-esm");
    if (uploads === undefined) throw new Error("uploads_fixture_missing");
    artifacts[artifacts.indexOf(uploads)] = {
      ...uploads,
      compatibleCore: ">=0.2.0 <0.3.0",
      brotliBytes: 20 * 1024 + 7,
    };

    const result = evaluate(artifacts);

    expect(result.issues).toEqual([
      "artifact_budget:duplicate:core-esm",
      "artifact_budget:missing:async-classic",
      "artifact_budget:compatible_core:uploads-esm",
      "artifact_budget:uploads-esm:+7",
    ]);
    expect(result.lines).toHaveLength(8);
  });
});

describe("browser benchmark provenance", () => {
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

  it("keeps the binding evaluator from comparing the prior artifact to itself", async () => {
    const source = await readFile(new URL("../scripts/check-budget.mjs", import.meta.url), "utf8");
    expect(source).not.toContain("evaluateBrowserBudget(baseline, baseline");
    expect(source).toContain("evaluateBrowserBudget(candidate, baseline");
  });

  it("rejects absent, stale, and artifact-mismatched binding candidates", async () => {
    const loaded = (await import("../scripts/check-budget.mjs")) as {
      readonly evaluateBindingEvidence: (
        baseline: {
          readonly recordedAt: string;
          readonly artifact: Readonly<{ sha256: string; brotliBytes: number }>;
        },
        candidate: {
          readonly recordedAt: string;
          readonly artifact: Readonly<{ sha256: string; brotliBytes: number }>;
          readonly methodology: Readonly<{ independentRuns: number }>;
        } | null,
        runtime: Readonly<{ sha256: string; brotliBytes: number }>,
        evaluate: (candidate: unknown, baseline: unknown) => unknown,
      ) => unknown;
    };
    const baseline = {
      artifact: { brotliBytes: 100, sha256: "a".repeat(64) },
      recordedAt: "2026-08-25T00:00:00.000Z",
    };
    const current = {
      artifact: { brotliBytes: 101, sha256: "b".repeat(64) },
      methodology: { independentRuns: 3 },
      recordedAt: "2026-08-26T00:00:00.000Z",
    };
    const runtime = current.artifact;

    expect(() => loaded.evaluateBindingEvidence(baseline, null, runtime, () => null)).toThrow(
      "browser_budget_candidate_missing",
    );
    expect(() =>
      loaded.evaluateBindingEvidence(
        baseline,
        { ...current, recordedAt: baseline.recordedAt },
        runtime,
        () => null,
      ),
    ).toThrow("browser_budget_candidate_stale");
    expect(() =>
      loaded.evaluateBindingEvidence(
        baseline,
        { ...current, artifact: { ...current.artifact, sha256: "c".repeat(64) } },
        runtime,
        () => null,
      ),
    ).toThrow("browser_budget_candidate_artifact_mismatch");
    expect(() =>
      loaded.evaluateBindingEvidence(
        baseline,
        { ...current, methodology: { independentRuns: 1 } },
        runtime,
        () => null,
      ),
    ).toThrow("browser_budget_candidate_runs");
  });
});
