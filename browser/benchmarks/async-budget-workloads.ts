export const ASYNC_BUDGET_DRIVER_MARKER = "SUPRNOVA_ASYNC_BUDGET_DRIVER_V1";

export interface AsyncRetainedCategories {
  readonly authorizationBytes: number;
  readonly identifierBytes: number;
  readonly pendingBytes: number;
  readonly pendingEvents: number;
  readonly pollTimers: number;
  readonly refreshSlots: number;
  readonly runtimeRecords: number;
}

export interface AsyncSampleSummary {
  readonly durationsMilliseconds: readonly number[];
  readonly p50Milliseconds: number;
  readonly p95Milliseconds: number;
  readonly sampleCount: number;
}

function boundedInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

/**
 * Estimates framework-owned retained bytes from one closed category record.
 * Native transport buffers, DOM nodes, and the current payload are absent by construction.
 */
export function estimateAsyncRetainedBytes(categories: AsyncRetainedCategories): number {
  const values: readonly number[] = [
    categories.authorizationBytes,
    categories.identifierBytes,
    categories.pendingBytes,
    categories.pendingEvents,
    categories.pollTimers,
    categories.refreshSlots,
    categories.runtimeRecords,
  ];
  if (values.some((value) => !boundedInteger(value))) {
    throw new Error("async_retained_accounting_invalid");
  }
  return (
    192 +
    categories.authorizationBytes +
    categories.identifierBytes +
    categories.pendingBytes +
    categories.pendingEvents * 128 +
    categories.pollTimers * 256 +
    categories.refreshSlots * 128 +
    categories.runtimeRecords * 192
  );
}

function rounded(value: number): number {
  return Math.round(value * 1_000_000) / 1_000_000;
}

/** Computes the deterministic nearest-rank summary used by async budget evidence. */
export function summarizeAsyncSamples(samples: readonly number[]): AsyncSampleSummary {
  if (samples.length === 0 || samples.some((sample) => !Number.isFinite(sample) || sample < 0)) {
    throw new Error("async_sample_set_invalid");
  }
  const copied = Object.freeze(samples.map(rounded));
  const ordered = [...copied].sort((left, right) => left - right);
  const percentile = (quantile: number): number => {
    const value = ordered[Math.max(0, Math.ceil(quantile * ordered.length) - 1)];
    if (value === undefined) throw new Error("async_sample_set_invalid");
    return value;
  };
  return Object.freeze({
    durationsMilliseconds: copied,
    p50Milliseconds: percentile(0.5),
    p95Milliseconds: percentile(0.95),
    sampleCount: copied.length,
  });
}
