export interface SampleSummary {
  readonly samplesMs: readonly number[];
  readonly sampleCount: number;
  readonly p50Ms: number;
  readonly p95Ms: number;
}

export type RegressionState = "improved" | "noise" | "observe" | "candidate" | "confirmed";

export interface RegressionClassification {
  readonly deltaPercent: number;
  readonly state: RegressionState;
}

function finiteNonnegative(value: number): boolean {
  return Number.isFinite(value) && value >= 0;
}

function rounded(value: number): number {
  return Math.round(value * 1_000_000) / 1_000_000;
}

export function percentile(samples: readonly number[], quantile: number): number {
  if (samples.length === 0) throw new Error("sample_set_empty");
  if (!Number.isFinite(quantile) || quantile <= 0 || quantile > 1) {
    throw new Error("quantile_invalid");
  }
  if (samples.some((sample) => !finiteNonnegative(sample))) throw new Error("sample_invalid");
  const ordered = [...samples].sort((left, right) => left - right);
  const index = Math.max(0, Math.ceil(quantile * ordered.length) - 1);
  const value = ordered[index];
  if (value === undefined) throw new Error("sample_set_empty");
  return rounded(value);
}

export function summarizeSamples(samples: readonly number[]): SampleSummary {
  if (samples.length === 0) throw new Error("sample_set_empty");
  if (samples.some((sample) => !finiteNonnegative(sample))) throw new Error("sample_invalid");
  const frozen = Object.freeze(samples.map(rounded));
  return Object.freeze({
    samplesMs: frozen,
    sampleCount: frozen.length,
    p50Ms: percentile(frozen, 0.5),
    p95Ms: percentile(frozen, 0.95),
  });
}

export function classifyP95Regression(
  baselineP95: number,
  independentRunP95s: readonly number[],
): RegressionClassification {
  if (!Number.isFinite(baselineP95) || baselineP95 <= 0) throw new Error("baseline_invalid");
  if (
    independentRunP95s.length === 0 ||
    independentRunP95s.length > 3 ||
    independentRunP95s.some((value) => !finiteNonnegative(value))
  ) {
    throw new Error("confirmation_runs_invalid");
  }
  const deltas = independentRunP95s.map(
    (candidate) => ((candidate - baselineP95) / baselineP95) * 100,
  );
  const deltaPercent = rounded(
    deltas.reduce((sum, candidate) => sum + candidate, 0) / deltas.length,
  );
  let state: RegressionState;
  if (deltaPercent < -5) state = "improved";
  else if (deltaPercent <= 5) state = "noise";
  else if (deltaPercent < 15) state = "observe";
  else if (deltas.length >= 3 && deltas.every((delta) => delta >= 15)) state = "confirmed";
  else state = "candidate";
  return Object.freeze({ deltaPercent, state });
}
