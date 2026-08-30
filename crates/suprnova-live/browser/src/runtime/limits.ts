export const RUNTIME_CONFIG_LIMITS = Object.freeze({
  maxBytes: 16_384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 2_048,
  minRequestTimeoutMs: 100,
  maxRequestTimeoutMs: 120_000,
  minResponseBytes: 1_024,
  maxResponseBytes: 4_194_304,
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
