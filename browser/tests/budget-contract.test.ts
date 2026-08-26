import { readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";

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

type ArtifactSizeBaseline = Readonly<{
  schemaVersion: 1;
  maximumUnreviewedIncreaseBasisPoints: 1500;
  methodology: Readonly<{
    buildCommand: "npm run build";
    compression: "brotli-quality-11";
    deterministic: true;
  }>;
  review: Readonly<{
    decision: string;
    rationale: string;
    recordedAt: string;
    sourceCommit: string;
  }>;
  roles: Readonly<
    Record<"async-esm" | "async-classic", Readonly<{ artifact: string; brotliBytes: number }>>
  >;
}>;

const EXPECTED = Object.freeze([
  ["core-esm", null],
  ["core-classic", null],
  ["stimulus-esm", 8 * 1024],
  ["stimulus-classic", 8 * 1024],
  ["uploads-esm", 20 * 1024],
  ["uploads-classic", 20 * 1024],
  ["async-esm", null],
  ["async-classic", null],
] as const);

const ARTIFACT_BASELINE = Object.freeze({
  maximumUnreviewedIncreaseBasisPoints: 1500 as const,
  methodology: Object.freeze({
    buildCommand: "npm run build" as const,
    compression: "brotli-quality-11" as const,
    deterministic: true as const,
  }),
  review: Object.freeze({
    decision: "iteration-004-task-6",
    rationale: "Last reviewed complete Task 6 production artifacts.",
    recordedAt: "2026-08-26T06:27:18-04:00",
    sourceCommit: "499eda2287f17d6a46c9b8c306df5791b1f671d8",
  }),
  roles: Object.freeze({
    "async-classic": Object.freeze({
      artifact: "suprnova-live.async.classic.js",
      brotliBytes: 14_155,
    }),
    "async-esm": Object.freeze({
      artifact: "suprnova-live.async.esm.js",
      brotliBytes: 16_356,
    }),
  }),
  schemaVersion: 1 as const,
}) satisfies ArtifactSizeBaseline;

async function evaluator(): Promise<
  Readonly<{
    evaluate: (
      assets: readonly ArtifactBudgetInput[],
      baseline: ArtifactSizeBaseline | null,
    ) => ArtifactBudgetEvaluation;
    validate: (value: unknown) => ArtifactSizeBaseline;
  }>
> {
  const moduleUrl = new URL("../scripts/check-budget.mjs", import.meta.url);
  const loaded = (await import(moduleUrl.href)) as {
    readonly evaluateArtifactBudgets: (
      assets: readonly ArtifactBudgetInput[],
      baseline: ArtifactSizeBaseline | null,
    ) => ArtifactBudgetEvaluation;
    readonly validateArtifactSizeBaseline: (value: unknown) => ArtifactSizeBaseline;
  };
  return Object.freeze({
    evaluate: loaded.evaluateArtifactBudgets,
    validate: loaded.validateArtifactSizeBaseline,
  });
}

function validArtifacts(): ArtifactBudgetInput[] {
  return EXPECTED.map(([role, ceiling]) => ({
    role,
    file:
      role === "async-esm"
        ? "suprnova-live.async.esm.js"
        : role === "async-classic"
          ? "suprnova-live.async.classic.js"
          : `${role}.js`,
    compatibleCore: ">=0.1.0 <0.2.0",
    brotliBytes:
      role === "async-esm" ? 17_987 : role === "async-classic" ? 15_773 : (ceiling ?? 128 * 1024),
  }));
}

describe("role-aware production artifact budgets", () => {
  it("reports exact async sizes without inventing a total download ceiling", async () => {
    const { evaluate } = await evaluator();
    const result = evaluate(validArtifacts(), ARTIFACT_BASELINE);

    expect(result.issues).toEqual([]);
    expect(result.lines).toContain(
      "artifact_budget role=async-esm bytes=17987 ceiling=none baseline=16356 unreviewed_increase=9.97% threshold=15%",
    );
    expect(result.lines).toContain(
      "artifact_budget role=async-classic bytes=15773 ceiling=none baseline=14155 unreviewed_increase=11.43% threshold=15%",
    );
    const formerlyArbitrary = validArtifacts();
    const asyncEsm = formerlyArbitrary.find(({ role }) => role === "async-esm");
    if (asyncEsm === undefined) throw new Error("async_esm_fixture_missing");
    formerlyArbitrary[formerlyArbitrary.indexOf(asyncEsm)] = {
      ...asyncEsm,
      brotliBytes: 16_385,
    };
    expect(evaluate(formerlyArbitrary, ARTIFACT_BASELINE).issues).toEqual([]);
  });

  it("fails only unreviewed async drift above fifteen percent", async () => {
    const { evaluate } = await evaluator();
    const artifacts = validArtifacts();
    const asyncEsm = artifacts.find(({ role }) => role === "async-esm");
    if (asyncEsm === undefined) throw new Error("async_esm_fixture_missing");
    artifacts[artifacts.indexOf(asyncEsm)] = { ...asyncEsm, brotliBytes: 18_810 };

    expect(evaluate(artifacts, ARTIFACT_BASELINE).issues).toContain(
      "artifact_budget:async-esm:unreviewed_regression:+2454",
    );
  });

  it("requires a separate reviewed baseline with closed provenance", async () => {
    const { evaluate, validate } = await evaluator();
    expect(() => evaluate(validArtifacts(), null)).toThrow("artifact_size_baseline_missing");
    expect(() => validate({ ...ARTIFACT_BASELINE, review: {} })).toThrow(
      "artifact_size_baseline_invalid",
    );
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        review: { ...ARTIFACT_BASELINE.review, decision: "" },
      }),
    ).toThrow("artifact_size_baseline_invalid");
    const candidateSizedRoles = Object.freeze({
      "async-classic": Object.freeze({
        artifact: "suprnova-live.async.classic.js",
        brotliBytes: 15_773,
      }),
      "async-esm": Object.freeze({
        artifact: "suprnova-live.async.esm.js",
        brotliBytes: 17_987,
      }),
    });
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        review: undefined,
        roles: candidateSizedRoles,
      }),
    ).toThrow("artifact_size_baseline_invalid");
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        roles: candidateSizedRoles,
        silentlySelfBaselined: true,
      }),
    ).toThrow("artifact_size_baseline_invalid");
  });

  it("reports duplicate, missing, incompatible, and over-budget roles together", async () => {
    const { evaluate } = await evaluator();
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

    const result = evaluate(artifacts, ARTIFACT_BASELINE);

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
  it("keeps ordinary clean-checkout budgets independent of ignored binding evidence", () => {
    const script = new URL("../scripts/check-budget.mjs", import.meta.url);
    const buildScript = new URL("../scripts/build.mjs", import.meta.url);
    const missing = new URL("../benchmarks/local/intentionally-absent.json", import.meta.url);
    const environment = {
      ...process.env,
      SUPRNOVA_LIVE_BROWSER_BUDGET_CANDIDATE: missing.pathname,
    };
    const built = spawnSync(process.execPath, [buildScript.pathname], {
      cwd: new URL("..", import.meta.url),
      encoding: "utf8",
      env: environment,
    });
    expect(`${built.stdout}${built.stderr}`).toBe("");
    expect(built.status).toBe(0);
    const ordinary = spawnSync(process.execPath, [script.pathname], {
      cwd: new URL("..", import.meta.url),
      encoding: "utf8",
      env: environment,
    });
    expect(`${ordinary.stdout}${ordinary.stderr}`).toContain("browser_binding=skipped");
    expect(ordinary.status).toBe(0);

    const binding = spawnSync(process.execPath, [script.pathname, "--binding"], {
      cwd: new URL("..", import.meta.url),
      encoding: "utf8",
      env: environment,
    });
    expect(binding.status).not.toBe(0);
    expect(`${binding.stdout}${binding.stderr}`).toContain("browser_budget_candidate_missing");
  }, 30_000);

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
