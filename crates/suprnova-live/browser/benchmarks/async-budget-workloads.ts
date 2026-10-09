export const ASYNC_BUDGET_DRIVER_MARKER = "SUPRNOVA_ASYNC_BUDGET_DRIVER_V1";

export interface AsyncHeapUsageSample {
  readonly backingStorageSize: number;
  readonly embedderHeapUsedSize: number;
  readonly usedSize: number;
}

export interface AsyncRetainedHeapMeasurement {
  readonly after: readonly AsyncHeapUsageSample[];
  readonly before: readonly AsyncHeapUsageSample[];
  readonly retainedBytes: number;
}

export interface AsyncSubscriptionOwnerObservation {
  readonly authorizationBytes: number;
  readonly currentPayloadBytes: number;
  readonly currentPayloadOwners: number;
  readonly id: string;
  readonly queuedPayloadBytes: number;
  readonly queuedPayloadOwners: number;
}

export interface AsyncPostWorkloadRetention {
  readonly baseline: readonly AsyncHeapUsageSample[];
  readonly cleanup: readonly AsyncHeapUsageSample[];
  readonly cleanupResidualBytes: number;
  readonly postWorkload: readonly AsyncHeapUsageSample[];
  readonly sharedAmortizedBytes: number;
  readonly sharedStructuralBytes: number;
  readonly subscriptions: readonly Readonly<
    AsyncSubscriptionOwnerObservation & {
      readonly ownedBytes: number;
      readonly retainedBytes: number;
    }
  >[];
  readonly totalOwnedBytes: number;
  readonly totalRetainedBytes: number;
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

function heapBytes(sample: AsyncHeapUsageSample): number {
  const values = [sample.backingStorageSize, sample.embedderHeapUsedSize, sample.usedSize];
  if (values.some((value) => !boundedInteger(value))) {
    throw new Error("async_heap_sample_invalid");
  }
  const total = values.reduce((sum, value) => sum + value, 0);
  if (!boundedInteger(total)) throw new Error("async_heap_sample_invalid");
  return total;
}

/** Conservatively derives one island's retained bytes from raw forced-GC heap samples. */
export function deriveRetainedHeapMeasurement(
  input: Readonly<{
    after: readonly AsyncHeapUsageSample[];
    before: readonly AsyncHeapUsageSample[];
  }>,
): AsyncRetainedHeapMeasurement {
  if (input.before.length === 0 || input.after.length !== input.before.length) {
    throw new Error("async_heap_sample_set_invalid");
  }
  const before = Object.freeze(input.before.map((sample) => Object.freeze({ ...sample })));
  const after = Object.freeze(input.after.map((sample) => Object.freeze({ ...sample })));
  const retainedBytes = Math.max(...after.map(heapBytes)) - Math.min(...before.map(heapBytes));
  if (!boundedInteger(retainedBytes)) throw new Error("async_heap_delta_invalid");
  return Object.freeze({ after, before, retainedBytes });
}

/** Attributes live post-workload heap without replacing any workloaded controller. */
export function derivePostWorkloadRetention(
  input: Readonly<{
    baseline: readonly AsyncHeapUsageSample[];
    cleanup: readonly AsyncHeapUsageSample[];
    postWorkload: readonly AsyncHeapUsageSample[];
    subscriptions: readonly AsyncSubscriptionOwnerObservation[];
  }>,
): AsyncPostWorkloadRetention {
  if (
    input.baseline.length === 0 ||
    input.cleanup.length !== input.baseline.length ||
    input.postWorkload.length !== input.baseline.length ||
    input.subscriptions.length === 0
  ) {
    throw new Error("async_heap_sample_set_invalid");
  }
  const baseline = Object.freeze(input.baseline.map((sample) => Object.freeze({ ...sample })));
  const cleanup = Object.freeze(input.cleanup.map((sample) => Object.freeze({ ...sample })));
  const postWorkload = Object.freeze(
    input.postWorkload.map((sample) => Object.freeze({ ...sample })),
  );
  const baselineMinimum = Math.min(...baseline.map(heapBytes));
  const totalRetainedBytes = Math.max(...postWorkload.map(heapBytes)) - baselineMinimum;
  const cleanupResidualBytes = Math.max(0, Math.max(...cleanup.map(heapBytes)) - baselineMinimum);
  if (!boundedInteger(totalRetainedBytes) || !boundedInteger(cleanupResidualBytes)) {
    throw new Error("async_heap_delta_invalid");
  }
  const observed = input.subscriptions.map((subscription) => {
    if (
      typeof subscription.id !== "string" ||
      subscription.id.length === 0 ||
      !boundedInteger(subscription.authorizationBytes) ||
      !boundedInteger(subscription.currentPayloadBytes) ||
      !boundedInteger(subscription.currentPayloadOwners) ||
      subscription.currentPayloadOwners > 1 ||
      !boundedInteger(subscription.queuedPayloadBytes) ||
      !boundedInteger(subscription.queuedPayloadOwners) ||
      subscription.queuedPayloadOwners > 1
    ) {
      throw new Error("async_owner_observation_invalid");
    }
    const ownedBytes =
      subscription.authorizationBytes +
      subscription.currentPayloadBytes +
      subscription.queuedPayloadBytes;
    if (!boundedInteger(ownedBytes)) throw new Error("async_owner_observation_invalid");
    return Object.freeze({ ...subscription, ownedBytes });
  });
  const totalOwnedBytes = observed.reduce((sum, subscription) => {
    const next = sum + subscription.ownedBytes;
    if (!boundedInteger(next)) throw new Error("async_owner_observation_invalid");
    return next;
  }, 0);
  const sharedStructuralBytes = Math.max(0, totalRetainedBytes - totalOwnedBytes);
  const sharedAmortizedBytes = Math.ceil(sharedStructuralBytes / observed.length);
  const subscriptions = Object.freeze(
    observed.map((subscription) => {
      const retainedBytes = sharedAmortizedBytes + subscription.ownedBytes;
      if (!boundedInteger(retainedBytes)) throw new Error("async_owner_observation_invalid");
      return Object.freeze({ ...subscription, retainedBytes });
    }),
  );
  return Object.freeze({
    baseline,
    cleanup,
    cleanupResidualBytes,
    postWorkload,
    sharedAmortizedBytes,
    sharedStructuralBytes,
    subscriptions,
    totalOwnedBytes,
    totalRetainedBytes,
  });
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
