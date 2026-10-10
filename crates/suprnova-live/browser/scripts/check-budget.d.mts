export interface ArtifactBudgetInput {
  readonly role: string;
  readonly file: string;
  readonly compatibleCore: string;
  readonly brotliBytes: number;
  readonly sha256?: string;
}

export interface ArtifactBudgetEvaluation {
  readonly lines: readonly string[];
  readonly issues: readonly string[];
}

export function evaluateArtifactBudgets(
  assets: readonly ArtifactBudgetInput[],
  baselineValue: unknown,
): ArtifactBudgetEvaluation;

export function validateArtifactSizeBaselineProvenance(
  value: unknown,
  repositoryRoot: string,
): unknown;

export function evaluateBindingEvidence(
  baseline: Readonly<{
    recordedAt: string;
    artifact: Readonly<{ sha256: string; brotliBytes: number }>;
  }>,
  candidate: Readonly<{
    recordedAt: string;
    artifact: Readonly<{ sha256: string; brotliBytes: number }>;
    methodology: Readonly<{ independentRuns: number }>;
  }> | null,
  runtime: Readonly<{ sha256: string; brotliBytes: number }>,
  evaluate: (
    candidate: unknown,
    baseline: unknown,
    options: Readonly<{ release: boolean }>,
  ) => unknown,
  release?: boolean,
): unknown;
