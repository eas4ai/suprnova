import { canonicalize, type JsonValue } from "../canonical.js";

const MAX_MODEL_VALUE_DEPTH = 32;
const MAX_MODEL_VALUE_NODES = 2_048;

export const MISSING: unique symbol = Symbol("suprnova.live.model.missing");
export type Missing = typeof MISSING;
export type ModelValue = JsonValue | Missing;

export function isMissing(value: ModelValue): value is Missing {
  return value === MISSING;
}

export function immutableModelValue(value: JsonValue): JsonValue {
  const budget = { remaining: MAX_MODEL_VALUE_NODES };
  return immutable(value, 0, budget);
}

export function modelValuesEqual(left: ModelValue, right: ModelValue): boolean {
  if (isMissing(left) || isMissing(right)) return left === right;
  return canonicalize(left) === canonicalize(right);
}

function immutable(value: JsonValue, depth: number, budget: { remaining: number }): JsonValue {
  if (depth > MAX_MODEL_VALUE_DEPTH || budget.remaining <= 0) {
    throw new Error("model_value_limit");
  }
  budget.remaining -= 1;
  if (Array.isArray(value)) {
    const values = value as readonly JsonValue[];
    return Object.freeze(values.map((item) => immutable(item, depth + 1, budget)));
  }
  if (value !== null && typeof value === "object") {
    const copy: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
    for (const [key, item] of Object.entries(value)) copy[key] = immutable(item, depth + 1, budget);
    return Object.freeze(copy);
  }
  return value;
}
