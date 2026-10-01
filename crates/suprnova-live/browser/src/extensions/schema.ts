import { canonicalize, type JsonValue } from "../canonical.js";

const MAX_SCHEMA_DEPTH = 12;
const MAX_SCHEMA_ENTRIES = 256;
const MAX_PAYLOAD_DEPTH = 16;
const MAX_PAYLOAD_ENTRIES = 256;
const MAX_PAYLOAD_BYTES = 16 * 1024;
const MAX_STRING_BYTES = 4 * 1024;
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

interface Budget {
  entries: number;
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

function compile(
  input: unknown,
  depth: number,
  budget: Budget,
  seen: WeakSet<object>,
): PayloadSchema {
  if (depth > MAX_SCHEMA_DEPTH || input === null || typeof input !== "object") {
    throw new PayloadValidationError("payload_schema_invalid");
  }
  if (seen.has(input)) throw new PayloadValidationError("payload_schema_cycle");
  seen.add(input);
  budget.entries += 1;
  if (budget.entries > MAX_SCHEMA_ENTRIES) {
    throw new PayloadValidationError("payload_schema_limit");
  }
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
        const maxBytes = candidate["maxBytes"] ?? MAX_STRING_BYTES;
        if (!boundedInteger(maxBytes, 0, MAX_STRING_BYTES)) {
          throw new PayloadValidationError("payload_schema_limit");
        }
        return Object.freeze({ type: "string", maxBytes });
      }
      case "array": {
        exactKeys(candidate, ["type", "items", "maxItems"]);
        const maxItems = candidate["maxItems"];
        if (!boundedInteger(maxItems, 0, MAX_PAYLOAD_ENTRIES)) {
          throw new PayloadValidationError("payload_schema_limit");
        }
        return Object.freeze({
          type: "array",
          items: compile(candidate["items"], depth + 1, budget, seen),
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
        if (
          names.length > MAX_SCHEMA_ENTRIES ||
          names.some((name) => !FIELD_NAME.test(name) || FORBIDDEN_FIELDS.has(name))
        ) {
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
          compiled[name] = compile(property, depth + 1, budget, seen);
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
  return compile(input, 0, { entries: 0 }, new WeakSet());
}

function stringBytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function visitJson(
  value: unknown,
  depth: number,
  budget: Budget,
  seen: WeakSet<object>,
): JsonValue {
  if (depth > MAX_PAYLOAD_DEPTH) throw new PayloadValidationError("payload_too_deep");
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
      throw new PayloadValidationError("payload_invalid_number");
    }
    return Object.is(value, -0) ? 0 : value;
  }
  if (typeof value === "string") {
    if (stringBytes(value) > MAX_STRING_BYTES) {
      throw new PayloadValidationError("payload_string_limit");
    }
    return value;
  }
  if (typeof value !== "object") throw new PayloadValidationError("payload_invalid_type");
  if (seen.has(value)) throw new PayloadValidationError("payload_cycle");
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      const result: JsonValue[] = [];
      for (const item of value) {
        budget.entries += 1;
        if (budget.entries > MAX_PAYLOAD_ENTRIES) {
          throw new PayloadValidationError("payload_entry_limit");
        }
        result.push(visitJson(item, depth + 1, budget, seen));
      }
      return Object.freeze(result);
    }
    const prototype = Object.getPrototypeOf(value) as unknown;
    if (prototype !== null && prototype !== Object.prototype) {
      throw new PayloadValidationError("payload_invalid_object");
    }
    const result = Object.create(null) as Record<string, JsonValue>;
    for (const key of Object.keys(value)) {
      if (FORBIDDEN_FIELDS.has(key)) throw new PayloadValidationError("payload_invalid_field");
      budget.entries += 1;
      if (budget.entries > MAX_PAYLOAD_ENTRIES) {
        throw new PayloadValidationError("payload_entry_limit");
      }
      result[key] = visitJson(Reflect.get(value, key), depth + 1, budget, seen);
    }
    return Object.freeze(result);
  } finally {
    seen.delete(value);
  }
}

export function boundedJsonValue(value: unknown): JsonValue {
  const normalized = visitJson(value, 0, { entries: 0 }, new WeakSet());
  if (stringBytes(canonicalize(normalized)) > MAX_PAYLOAD_BYTES) {
    throw new PayloadValidationError("payload_byte_limit");
  }
  return normalized;
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
      if (typeof value !== "string" || stringBytes(value) > (schema.maxBytes ?? MAX_STRING_BYTES)) {
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
