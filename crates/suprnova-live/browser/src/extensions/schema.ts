import type { JsonValue } from "../canonical.js";
import { JSON_DEPTH_CEILING } from "../limits.js";

// An extension payload is either an effect the server returned or a call the
// application makes; the application's own schema (`maxBytes`, `maxItems`)
// is the size policy, and there is no runtime-wide byte, entry or string cap
// on top of it. Schemas and payloads are walked recursively, so their depth is
// guarded at the ceiling no server configuration exceeds.
const MAX_SCHEMA_DEPTH = JSON_DEPTH_CEILING;
const MAX_PAYLOAD_DEPTH = JSON_DEPTH_CEILING;
const FIELD_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/u;
const FORBIDDEN_FIELDS = new Set(["__proto__", "constructor", "prototype"]);

type PrimitivePayloadSchema =
  | Readonly<{ type: "null" }>
  | Readonly<{ type: "boolean" }>
  | Readonly<{ type: "number" }>
  | Readonly<{ type: "integer" }>
  | Readonly<{ type: "string"; maxBytes?: number }>;

export type PayloadSchema =
  | PrimitivePayloadSchema
  | Readonly<{ type: "array"; items: PayloadSchema; maxItems: number }>
  | Readonly<{
      type: "object";
      properties: Readonly<Record<string, PayloadSchema>>;
      required: readonly string[];
      additionalProperties: false;
    }>;

export class PayloadValidationError extends Error {
  constructor(readonly code: string) {
    super(code);
    this.name = "PayloadValidationError";
  }
}

function exactKeys(value: object, allowed: readonly string[]): void {
  const keys = Object.keys(value);
  if (keys.some((key) => !allowed.includes(key))) {
    throw new PayloadValidationError("payload_schema_shape");
  }
}

function boundedInteger(value: unknown, minimum: number, maximum: number): value is number {
  return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum;
}

function compile(input: unknown, depth: number, seen: WeakSet<object>): PayloadSchema {
  if (depth > MAX_SCHEMA_DEPTH || input === null || typeof input !== "object") {
    throw new PayloadValidationError("payload_schema_invalid");
  }
  if (seen.has(input)) throw new PayloadValidationError("payload_schema_cycle");
  seen.add(input);
  const candidate = input as Readonly<Record<string, unknown>>;
  try {
    switch (candidate["type"]) {
      case "null":
      case "boolean":
      case "number":
      case "integer":
        exactKeys(candidate, ["type"]);
        return Object.freeze({ type: candidate["type"] });
      case "string": {
        exactKeys(candidate, ["type", "maxBytes"]);
        const maxBytes = candidate["maxBytes"];
        if (maxBytes === undefined) return Object.freeze({ type: "string" });
        if (!boundedInteger(maxBytes, 0, Number.MAX_SAFE_INTEGER)) {
          throw new PayloadValidationError("payload_schema_limit");
        }
        return Object.freeze({ type: "string", maxBytes });
      }
      case "array": {
        exactKeys(candidate, ["type", "items", "maxItems"]);
        const maxItems = candidate["maxItems"];
        if (!boundedInteger(maxItems, 0, Number.MAX_SAFE_INTEGER)) {
          throw new PayloadValidationError("payload_schema_limit");
        }
        return Object.freeze({
          type: "array",
          items: compile(candidate["items"], depth + 1, seen),
          maxItems,
        });
      }
      case "object": {
        exactKeys(candidate, ["type", "properties", "required", "additionalProperties"]);
        const requiredInput = candidate["required"];
        if (candidate["additionalProperties"] !== false || !Array.isArray(requiredInput)) {
          throw new PayloadValidationError("payload_schema_invalid");
        }
        if (!requiredInput.every((name) => typeof name === "string")) {
          throw new PayloadValidationError("payload_schema_invalid");
        }
        const properties = candidate["properties"];
        if (properties === null || typeof properties !== "object" || Array.isArray(properties)) {
          throw new PayloadValidationError("payload_schema_invalid");
        }
        const propertyRecord = properties as Readonly<Record<string, unknown>>;
        const names = Object.keys(propertyRecord);
        if (names.some((name) => !FIELD_NAME.test(name) || FORBIDDEN_FIELDS.has(name))) {
          throw new PayloadValidationError("payload_schema_invalid");
        }
        const required: string[] = [...requiredInput];
        if (
          required.length > names.length ||
          new Set(required).size !== required.length ||
          required.some((name) => !names.includes(name))
        ) {
          throw new PayloadValidationError("payload_schema_invalid");
        }
        const compiled = Object.create(null) as Record<string, PayloadSchema>;
        for (const name of names) {
          const property = propertyRecord[name];
          if (property === undefined) throw new PayloadValidationError("payload_schema_invalid");
          compiled[name] = compile(property, depth + 1, seen);
        }
        return Object.freeze({
          type: "object",
          properties: Object.freeze(compiled),
          required: Object.freeze(required),
          additionalProperties: false,
        });
      }
      default:
        throw new PayloadValidationError("payload_schema_invalid");
    }
  } finally {
    seen.delete(input);
  }
}

export function compilePayloadSchema(input: PayloadSchema): PayloadSchema {
  return compile(input, 0, new WeakSet());
}

function stringBytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function visitJson(value: unknown, depth: number, seen: WeakSet<object>): JsonValue {
  if (depth > MAX_PAYLOAD_DEPTH) throw new PayloadValidationError("payload_too_deep");
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
      throw new PayloadValidationError("payload_invalid_number");
    }
    return Object.is(value, -0) ? 0 : value;
  }
  if (typeof value === "string") return value;
  if (typeof value !== "object") throw new PayloadValidationError("payload_invalid_type");
  if (seen.has(value)) throw new PayloadValidationError("payload_cycle");
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      const result: JsonValue[] = [];
      for (const item of value) result.push(visitJson(item, depth + 1, seen));
      return Object.freeze(result);
    }
    const prototype = Object.getPrototypeOf(value) as unknown;
    if (prototype !== null && prototype !== Object.prototype) {
      throw new PayloadValidationError("payload_invalid_object");
    }
    const result = Object.create(null) as Record<string, JsonValue>;
    for (const key of Object.keys(value)) {
      if (FORBIDDEN_FIELDS.has(key)) throw new PayloadValidationError("payload_invalid_field");
      result[key] = visitJson(Reflect.get(value, key), depth + 1, seen);
    }
    return Object.freeze(result);
  } finally {
    seen.delete(value);
  }
}

export function boundedJsonValue(value: unknown): JsonValue {
  return visitJson(value, 0, new WeakSet());
}

function validateShape(schema: PayloadSchema, value: JsonValue): JsonValue {
  switch (schema.type) {
    case "null":
      if (value !== null) throw new PayloadValidationError("payload_schema_mismatch");
      return value;
    case "boolean":
      if (typeof value !== "boolean") throw new PayloadValidationError("payload_schema_mismatch");
      return value;
    case "number":
      if (typeof value !== "number") throw new PayloadValidationError("payload_schema_mismatch");
      return value;
    case "integer":
      if (typeof value !== "number" || !Number.isSafeInteger(value)) {
        throw new PayloadValidationError("payload_schema_mismatch");
      }
      return value;
    case "string":
      if (
        typeof value !== "string" ||
        (schema.maxBytes !== undefined && stringBytes(value) > schema.maxBytes)
      ) {
        throw new PayloadValidationError("payload_schema_mismatch");
      }
      return value;
    case "array":
      if (!Array.isArray(value) || value.length > schema.maxItems) {
        throw new PayloadValidationError("payload_schema_mismatch");
      }
      return Object.freeze(
        (value as readonly JsonValue[]).map((item) => validateShape(schema.items, item)),
      );
    case "object": {
      if (value === null || typeof value !== "object" || Array.isArray(value)) {
        throw new PayloadValidationError("payload_schema_mismatch");
      }
      const object = value as Readonly<Record<string, JsonValue>>;
      const names = Object.keys(object);
      if (
        schema.required.some((name) => !Object.prototype.hasOwnProperty.call(object, name)) ||
        names.some((name) => !Object.prototype.hasOwnProperty.call(schema.properties, name))
      ) {
        throw new PayloadValidationError("payload_schema_mismatch");
      }
      const result = Object.create(null) as Record<string, JsonValue>;
      for (const name of names) {
        const property = schema.properties[name];
        if (property === undefined) throw new PayloadValidationError("payload_schema_mismatch");
        result[name] = validateShape(property, object[name] ?? null);
      }
      return Object.freeze(result);
    }
  }
}

export function validatePayload(schema: PayloadSchema, value: unknown): JsonValue {
  return validateShape(schema, boundedJsonValue(value));
}
