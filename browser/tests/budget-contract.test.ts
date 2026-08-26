import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { argumentsFrom, type BrowserBudgetArguments } from "../scripts/run-browser-budget.mjs";
import {
  PRODUCTION_BUILD_HOOK_TIMEOUT_MS,
  withProductionBuildLock,
} from "./support/production-build.js";

type ArtifactBudgetInput = Readonly<{
  role: string;
  file: string;
  compatibleCore: string;
  brotliBytes: number;
  sha256?: string;
}>;

type ArtifactBudgetEvaluation = Readonly<{
  lines: readonly string[];
  issues: readonly string[];
}>;

type ArtifactSizeBaseline = Readonly<{
  schemaVersion: 2;
  maximumUnreviewedIncreaseBasisPoints: 1500;
  methodology: Readonly<{
    buildCommand: "npm run build";
    compression: "brotli-quality-11";
    deterministic: true;
  }>;
  history: readonly Readonly<{
    review: Readonly<{
      decision: string;
      rationale: string;
      recordedAt: string;
      sourceCommit: string;
      sourceDecision?: string;
      sourceDecisionPath?: string;
    }>;
    roles: Readonly<
      Record<
        "async-esm" | "async-classic",
        Readonly<{ artifact: string; brotliBytes: number; sha256?: string }>
      >
    >;
  }>[];
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
  history: Object.freeze([
    Object.freeze({
      review: Object.freeze({
        decision: "iteration-004-task-6",
        rationale:
          "Last reviewed complete Task 6 deterministic production artifacts before polling implementation.",
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
    }),
    Object.freeze({
      review: Object.freeze({
        decision: "iteration-004-task-7-membership-budget-policy",
        rationale:
          "Lifecycle-aware committed-morph reconciliation, event-driven offline and hidden storm prevention, public freshness semantics, and exact membership-proof deferral are explicitly reviewed correctness growth.",
        recordedAt: "2026-08-26T10:56:00-04:00",
        sourceCommit: "57eb8c260abe44f9aacd8c2cc03b1a54f3ceec61",
        sourceDecision: "iteration-004-task-7-membership-budget-policy",
        sourceDecisionPath: "docs/specs/suprnova-live/19-developer-tooling-and-testing.md",
      }),
      roles: Object.freeze({
        "async-classic": Object.freeze({
          artifact: "suprnova-live.async.classic.js",
          brotliBytes: 16_459,
          sha256: "23effe66a533065544c19bef1c88819466ba2f514e28b0271dc14c1494e82b5e",
        }),
        "async-esm": Object.freeze({
          artifact: "suprnova-live.async.esm.js",
          brotliBytes: 18_713,
          sha256: "e030eb202f90312d002b2531dae8f42d12621910f800ff4ae29389f0dc9064ca",
        }),
      }),
    }),
  ]),
  schemaVersion: 2 as const,
}) satisfies ArtifactSizeBaseline;

function baselineEntry(index: number): (typeof ARTIFACT_BASELINE.history)[number] {
  const entry = ARTIFACT_BASELINE.history[index];
  if (entry === undefined) throw new Error("artifact_baseline_fixture_missing");
  return entry;
}

const TASK6_ARTIFACT_BASELINE = baselineEntry(0);
const CURRENT_ARTIFACT_BASELINE = baselineEntry(ARTIFACT_BASELINE.history.length - 1);
const REVIEWED_ARTIFACT_BASELINE = baselineEntry(1);
const TASK6_ONLY_ARTIFACT_BASELINE = Object.freeze({
  ...ARTIFACT_BASELINE,
  history: Object.freeze([TASK6_ARTIFACT_BASELINE]),
});

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
    ...(role === "async-esm" || role === "async-classic" ? { sha256: "d".repeat(64) } : {}),
    brotliBytes:
      role === "async-esm"
        ? CURRENT_ARTIFACT_BASELINE.roles["async-esm"].brotliBytes
        : role === "async-classic"
          ? CURRENT_ARTIFACT_BASELINE.roles["async-classic"].brotliBytes
          : (ceiling ?? 128 * 1024),
  }));
}

describe("role-aware production artifact budgets", () => {
  it("has no arbitrary total download ceiling", async () => {
    const { evaluate } = await evaluator();
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
    const excess = Math.floor(asyncEsm.brotliBytes * 1.15) + 1;
    artifacts[artifacts.indexOf(asyncEsm)] = { ...asyncEsm, brotliBytes: excess };

    expect(evaluate(artifacts, ARTIFACT_BASELINE).issues).toEqual([
      `artifact_budget:async-esm:unreviewed_regression:+${String(excess - asyncEsm.brotliBytes)}`,
    ]);
  });

  it("accepts a changed candidate hash below the reviewed drift threshold", async () => {
    const { evaluate } = await evaluator();
    const artifacts = validArtifacts();
    const asyncEsm = artifacts.find(({ role }) => role === "async-esm");
    if (asyncEsm === undefined) throw new Error("async_esm_fixture_missing");
    artifacts[artifacts.indexOf(asyncEsm)] = {
      ...asyncEsm,
      brotliBytes: Math.floor(asyncEsm.brotliBytes * 1.1),
      sha256: "c".repeat(64),
    };

    expect(evaluate(artifacts, ARTIFACT_BASELINE).issues).toEqual([]);
  });

  it("requires append-only reviewed history with closed provenance", async () => {
    const { evaluate, validate } = await evaluator();
    const reviewed = {
      ...ARTIFACT_BASELINE,
      history: [TASK6_ARTIFACT_BASELINE, REVIEWED_ARTIFACT_BASELINE],
    };
    expect(validate(reviewed)).toEqual(reviewed);
    expect(() => evaluate(validArtifacts(), null)).toThrow("artifact_size_baseline_missing");
    expect(() => validate({ ...ARTIFACT_BASELINE, history: [] })).toThrow(
      "artifact_size_baseline_invalid",
    );
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        history: [
          {
            ...TASK6_ARTIFACT_BASELINE,
            roles: {
              ...TASK6_ARTIFACT_BASELINE.roles,
              "async-esm": {
                ...TASK6_ARTIFACT_BASELINE.roles["async-esm"],
                brotliBytes: 16_357,
              },
            },
          },
          REVIEWED_ARTIFACT_BASELINE,
        ],
      }),
    ).toThrow("artifact_size_baseline_invalid");
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        history: [
          TASK6_ARTIFACT_BASELINE,
          {
            ...CURRENT_ARTIFACT_BASELINE,
            review: {
              ...CURRENT_ARTIFACT_BASELINE.review,
              sourceDecision: undefined,
            },
          },
        ],
      }),
    ).toThrow("artifact_size_baseline_invalid");
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        history: [TASK6_ARTIFACT_BASELINE, TASK6_ARTIFACT_BASELINE],
      }),
    ).toThrow("artifact_size_baseline_invalid");
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        history: [
          TASK6_ARTIFACT_BASELINE,
          {
            ...REVIEWED_ARTIFACT_BASELINE,
            review: {
              ...REVIEWED_ARTIFACT_BASELINE.review,
              sourceDecisionPath: undefined,
            },
          },
        ],
      }),
    ).toThrow("artifact_size_baseline_invalid");
    expect(() =>
      validate({
        ...ARTIFACT_BASELINE,
        history: [
          TASK6_ARTIFACT_BASELINE,
          {
            ...REVIEWED_ARTIFACT_BASELINE,
            roles: {
              ...REVIEWED_ARTIFACT_BASELINE.roles,
              "async-classic": {
                artifact: "suprnova-live.async.classic.js",
                brotliBytes: 16_420,
              },
            },
          },
        ],
      }),
    ).toThrow("artifact_size_baseline_invalid");
  });

  it("requires the review decision and code commit to predate the baseline append", async () => {
    const repository = await mkdtemp(join(tmpdir(), "suprnova-live-artifact-provenance-"));
    const baselineRelative = "browser/benchmarks/baselines/artifact-size-v1.json";
    const decisionRelative = "docs/specs/suprnova-live/19-developer-tooling-and-testing.md";
    const runGit = (...arguments_: string[]): string => {
      const result = spawnSync("git", arguments_, { cwd: repository, encoding: "utf8" });
      if (result.status !== 0) {
        throw new Error(`${result.stdout}${result.stderr}`);
      }
      return result.stdout.trim();
    };
    try {
      await mkdir(join(repository, "browser/benchmarks/baselines"), { recursive: true });
      await mkdir(join(repository, "docs/specs/suprnova-live"), { recursive: true });
      await writeFile(
        join(repository, baselineRelative),
        `${JSON.stringify(TASK6_ONLY_ARTIFACT_BASELINE, null, 2)}\n`,
      );
      await writeFile(
        join(repository, decisionRelative),
        "Decision ID: iteration-004-task-7-membership-budget-policy\n",
      );
      runGit("init", "-q");
      runGit("config", "user.email", "fixture@example.test");
      runGit("config", "user.name", "Fixture");
      runGit("add", ".");
      runGit("commit", "-qm", "record prior review decision");
      const sourceCommit = runGit("rev-parse", "HEAD");
      const reviewed = {
        ...ARTIFACT_BASELINE,
        history: [
          TASK6_ARTIFACT_BASELINE,
          {
            ...REVIEWED_ARTIFACT_BASELINE,
            review: {
              ...REVIEWED_ARTIFACT_BASELINE.review,
              sourceCommit,
            },
          },
        ],
      };
      await writeFile(join(repository, baselineRelative), `${JSON.stringify(reviewed, null, 2)}\n`);
      runGit("add", baselineRelative);
      runGit("commit", "-qm", "append reviewed baseline");
      const appendCommit = runGit("rev-parse", "HEAD");
      const loaded = (await import("../scripts/check-budget.mjs")) as {
        readonly validateArtifactSizeBaselineProvenance: (
          value: unknown,
          repositoryRoot: string,
        ) => unknown;
      };

      expect(loaded.validateArtifactSizeBaselineProvenance(reviewed, repository)).toEqual(reviewed);
      expect(() =>
        loaded.validateArtifactSizeBaselineProvenance(
          {
            ...reviewed,
            history: [
              TASK6_ARTIFACT_BASELINE,
              {
                ...reviewed.history[1],
                review: { ...reviewed.history[1]?.review, sourceCommit: appendCommit },
              },
            ],
          },
          repository,
        ),
      ).toThrow("artifact_size_baseline_provenance_invalid");
      expect(() =>
        loaded.validateArtifactSizeBaselineProvenance(TASK6_ONLY_ARTIFACT_BASELINE, repository),
      ).toThrow("artifact_size_baseline_provenance_invalid");
    } finally {
      await rm(repository, { force: true, recursive: true });
    }
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
  it(
    "keeps ordinary clean-checkout budgets independent of ignored binding evidence",
    async () => {
      const checkedBaseline = JSON.parse(
        await readFile(
          new URL("../benchmarks/baselines/artifact-size-v1.json", import.meta.url),
          "utf8",
        ),
      ) as ArtifactSizeBaseline;
      await withProductionBuildLock(() => {
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
        const ordinaryOutput = `${ordinary.stdout}${ordinary.stderr}`;
        if (checkedBaseline.history.length === 1) {
          expect(ordinary.status).not.toBe(0);
          expect(ordinaryOutput).toContain(
            "artifact_budget_failed:artifact_budget:async-classic:unreviewed_regression:",
          );
          expect(ordinaryOutput).not.toContain("baseline_hash");
          return;
        }
        expect(ordinaryOutput).toContain("browser_binding=skipped");
        expect(ordinary.status).toBe(0);

        const binding = spawnSync(process.execPath, [script.pathname, "--binding"], {
          cwd: new URL("..", import.meta.url),
          encoding: "utf8",
          env: environment,
        });
        expect(binding.status).not.toBe(0);
        expect(`${binding.stdout}${binding.stderr}`).toContain("browser_budget_candidate_missing");
      });
    },
    PRODUCTION_BUILD_HOOK_TIMEOUT_MS * 2,
  );

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
