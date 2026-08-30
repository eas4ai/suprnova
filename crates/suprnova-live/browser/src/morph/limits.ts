import type { MorphLimits } from "./types.js";

export const DEFAULT_MORPH_LIMITS: MorphLimits = Object.freeze({
  maxHtmlBytes: 1_048_576,
  maxNodes: 10_000,
  maxDepth: 128,
  maxAttributes: 65_536,
  maxAttributesPerElement: 256,
  maxKeys: 10_000,
  maxKeyBytes: 128,
  maxHookCalls: 200_000,
  deadlineMs: 1_000,
});

export function validateMorphLimits(limits: MorphLimits): void {
  for (const value of Object.values(limits)) {
    if (!Number.isSafeInteger(value) || value <= 0) throw new Error("morph_limits_invalid");
  }
  if (limits.maxAttributesPerElement > limits.maxAttributes) {
    throw new Error("morph_limits_invalid");
  }
}
