/// Bounds on the shape of the bootstrap's configuration element itself: the
/// in-flight request structure of one island and the bootstrap options. The
/// page limits it carries are the server's values (`../limits.ts`), never
/// these. Each bound here is tied to a resource: a request timeout above
/// `setTimeout`'s largest delay fires at once, and the queue and parallel
/// bounds are the sizes the scheduler's per-island structures are built for;
/// the server validates its configuration against the same ranges.
export const RUNTIME_CONFIG_LIMITS = Object.freeze({
  minRequestTimeoutMs: 1,
  maxRequestTimeoutMs: 2_147_483_647,
  maxQueuedPerIsland: 64,
  maxParallelPerIsland: 8,
  maxAllowedOrigins: 32,
  maxAssetIdentityUnits: 128,
} as const);

export const MAX_DIAGNOSTIC_ENTRIES = 1_024;
export const MAX_DIAGNOSTIC_SEQUENCE = 4_294_967_295;

export function boundedInteger(value: unknown, minimum: number, maximum: number): value is number {
  return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum;
}
