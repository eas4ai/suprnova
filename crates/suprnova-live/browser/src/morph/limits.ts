import { SERVER_DEFAULT_LIMITS, type LiveLimits } from "../limits.js";
import type { MorphLimits } from "./types.js";

/// The morph's share of the page limits. The runtime builds it from the
/// server's configuration (`LiveLimits`); the island HTML limit is the same
/// setting the protocol validator applies to the render.
export function morphLimitsFrom(limits: LiveLimits): MorphLimits {
  return Object.freeze({
    maxHtmlBytes: limits.maxHtmlBytes,
    maxNodes: limits.morphMaxNodes,
    maxDepth: limits.morphMaxDepth,
    maxAttributes: limits.morphMaxAttributes,
    maxAttributesPerElement: limits.morphMaxAttributesPerElement,
    maxKeys: limits.morphMaxKeys,
    deadlineMs: limits.morphDeadlineMs,
  });
}

/// The server's default morph limits, for callers that run without the
/// configuration element. Never tighter than the server's defaults.
export const DEFAULT_MORPH_LIMITS: MorphLimits = morphLimitsFrom(SERVER_DEFAULT_LIMITS);

export function validateMorphLimits(limits: MorphLimits): void {
  for (const [name, value] of Object.entries(limits)) {
    const minimum = name === "deadlineMs" ? 0 : 1;
    if (!Number.isSafeInteger(value) || (value as number) < minimum) {
      throw new Error("morph_limits_invalid");
    }
  }
  if (limits.maxAttributesPerElement > limits.maxAttributes) {
    throw new Error("morph_limits_invalid");
  }
}
