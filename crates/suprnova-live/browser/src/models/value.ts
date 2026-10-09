import { canonicalize, type JsonValue } from "../canonical.js";
import { JSON_DEPTH_CEILING } from "../limits.js";

// A model value is copied recursively, so its depth is guarded at the ceiling no
// server configuration exceeds; its size is checked against the server's
// request limits when a request carries it.
const MAX_MODEL_VALUE_DEPTH = JSON_DEPTH_CEILING;

export const MISSING: unique symbol = Symbol("suprnova.live.model.missing");
export type Missing = typeof MISSING;
export type ModelValue = JsonValue | Missing;

export function isMissing(value: ModelValue): value is Missing {
  return value === MISSING;
}

export function immutableModelValue(value: JsonValue): JsonValue {
  return immutable(value, 0);
}

export function modelValuesEqual(left: ModelValue, right: ModelValue): boolean {
  if (isMissing(left) || isMissing(right)) return left === right;
  return canonicalize(left) === canonicalize(right);
}

function immutable(value: JsonValue, depth: number): JsonValue {
  if (depth > MAX_MODEL_VALUE_DEPTH) throw new Error("model_value_limit");
  if (Array.isArray(value)) {
    const values = value as readonly JsonValue[];
    return Object.freeze(values.map((item) => immutable(item, depth + 1)));
  }
  if (value !== null && typeof value === "object") {
    const copy: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
    for (const [key, item] of Object.entries(value)) copy[key] = immutable(item, depth + 1);
    return Object.freeze(copy);
  }
  return value;
}
