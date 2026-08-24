import { describe, expect, it } from "vitest";

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
