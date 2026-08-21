export function hasOwn(value: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

export function asRecord(value: unknown): Readonly<Record<string, unknown>> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("expected_object");
  }
  const result = Object.create(null) as Record<string, unknown>;
  for (const key of Object.keys(value)) {
    result[key] = Reflect.get(value, key);
  }
  return result;
}

export function asArray(value: unknown): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw new TypeError("expected_array");
  }
  return value;
}

export function asString(value: unknown): string {
  if (typeof value !== "string") {
    throw new TypeError("expected_string");
  }
  return value;
}

export function asNumber(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TypeError("expected_number");
  }
  return value;
}

export function fixtureCases(value: unknown): readonly Readonly<Record<string, unknown>>[] {
  const root = asRecord(value);
  if (asNumber(root["schema_version"]) !== 1) {
    throw new TypeError("unsupported_fixture_schema");
  }
  return asArray(root["cases"]).map((item) => asRecord(item));
}

export function asJsonValue(value: unknown): JsonValue {
  if (
    value === null ||
    typeof value === "boolean" ||
    typeof value === "string" ||
    (typeof value === "number" && Number.isFinite(value))
  ) {
    return value;
  }
  if (Array.isArray(value)) return value.map(asJsonValue);
  const source = asRecord(value);
  const result = Object.create(null) as Record<string, JsonValue>;
  for (const [key, item] of Object.entries(source)) result[key] = asJsonValue(item);
  return result;
}

export function requireExactKeys(
  value: Readonly<Record<string, unknown>>,
  required: readonly string[],
  optional: readonly string[] = [],
): void {
  const allowed = new Set([...required, ...optional]);
  if (
    required.some((key) => !hasOwn(value, key)) ||
    Object.keys(value).some((key) => !allowed.has(key))
  ) {
    throw new TypeError("invalid_envelope");
  }
}
import type { JsonValue } from "./canonical.js";
