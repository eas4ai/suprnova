export type JsonValue = null | boolean | number | string | JsonArray | JsonObject;

export interface JsonArray extends ReadonlyArray<JsonValue> {
  readonly [index: number]: JsonValue;
}

export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export interface CanonicalLimits {
  readonly maxBytes: number;
  readonly maxDepth: number;
  readonly maxEntries: number;
  readonly maxStringBytes: number;
}

export const DEFAULT_CANONICAL_LIMITS: CanonicalLimits = {
  maxBytes: 64 * 1024,
  maxDepth: 32,
  maxEntries: 2048,
  maxStringBytes: 16 * 1024,
};

export class CanonicalError extends Error {
  public constructor(public readonly code: string) {
    super(code);
    this.name = "CanonicalError";
  }
}

class Parser {
  private index = 0;
  private entries = 0;
  private readonly bytes: number;

  public constructor(
    private readonly text: string,
    private readonly limits: CanonicalLimits,
  ) {
    this.bytes = new TextEncoder().encode(text).byteLength;
    if (this.bytes > limits.maxBytes) throw new CanonicalError("input_too_large");
  }

  public parse(): JsonValue {
    this.space();
    const value = this.value(0);
    this.space();
    if (this.index !== this.text.length) throw new CanonicalError("invalid_json");
    return value;
  }

  private value(depth: number): JsonValue {
    if (depth > this.limits.maxDepth) throw new CanonicalError("input_too_deep");
    const current = this.text[this.index];
    if (current === '"') return this.string();
    if (current === "{") return this.object(depth + 1);
    if (current === "[") return this.array(depth + 1);
    if (this.text.startsWith("true", this.index)) return this.literal("true", true);
    if (this.text.startsWith("false", this.index)) return this.literal("false", false);
    if (this.text.startsWith("null", this.index)) return this.literal("null", null);
    return this.number();
  }

  private literal<T extends JsonValue>(token: string, value: T): T {
    this.index += token.length;
    return value;
  }

  private string(): string {
    const start = this.index;
    this.index += 1;
    let escaped = false;
    while (this.index < this.text.length) {
      const character = this.text[this.index];
      if (character === undefined) break;
      if (!escaped && character === '"') {
        this.index += 1;
        const raw = this.text.slice(start, this.index);
        let decoded: unknown;
        try {
          decoded = JSON.parse(raw);
        } catch {
          throw new CanonicalError("invalid_json");
        }
        if (typeof decoded !== "string") throw new CanonicalError("invalid_json");
        if (new TextEncoder().encode(decoded).byteLength > this.limits.maxStringBytes) {
          throw new CanonicalError("string_too_long");
        }
        return decoded;
      }
      if (!escaped && character.charCodeAt(0) < 0x20) {
        throw new CanonicalError("invalid_json");
      }
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      this.index += 1;
    }
    throw new CanonicalError("invalid_json");
  }

  private number(): number {
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/u.exec(this.text.slice(this.index));
    const token = match?.[0];
    if (token === undefined) throw new CanonicalError("invalid_json");
    this.index += token.length;
    const value = Number(token);
    if (!Number.isFinite(value)) throw new CanonicalError("invalid_number");
    if (!token.includes(".") && !/[eE]/u.test(token) && !Number.isSafeInteger(value)) {
      throw new CanonicalError("invalid_number");
    }
    return Object.is(value, -0) ? 0 : value;
  }

  private array(depth: number): readonly JsonValue[] {
    this.index += 1;
    this.space();
    const values: JsonValue[] = [];
    if (this.text[this.index] === "]") {
      this.index += 1;
      return values;
    }
    for (;;) {
      this.bumpEntry();
      values.push(this.value(depth));
      this.space();
      const separator = this.text[this.index];
      this.index += 1;
      if (separator === "]") return values;
      if (separator !== ",") throw new CanonicalError("invalid_json");
      this.space();
    }
  }

  private object(depth: number): Readonly<Record<string, JsonValue>> {
    this.index += 1;
    this.space();
    const values: Record<string, JsonValue> = {};
    const keys = new Set<string>();
    if (this.text[this.index] === "}") {
      this.index += 1;
      return values;
    }
    for (;;) {
      if (this.text[this.index] !== '"') throw new CanonicalError("invalid_json");
      const key = this.string();
      if (keys.has(key)) throw new CanonicalError("duplicate_key");
      keys.add(key);
      this.bumpEntry();
      this.space();
      if (this.text[this.index] !== ":") throw new CanonicalError("invalid_json");
      this.index += 1;
      this.space();
      values[key] = this.value(depth);
      this.space();
      const separator = this.text[this.index];
      this.index += 1;
      if (separator === "}") return values;
      if (separator !== ",") throw new CanonicalError("invalid_json");
      this.space();
    }
  }

  private bumpEntry(): void {
    this.entries += 1;
    if (this.entries > this.limits.maxEntries) throw new CanonicalError("too_many_entries");
  }

  private space(): void {
    while (/\s/u.test(this.text[this.index] ?? "")) this.index += 1;
  }
}

export function parseCanonicalJson(
  text: string,
  limits: CanonicalLimits = DEFAULT_CANONICAL_LIMITS,
): JsonValue {
  return new Parser(text, limits).parse();
}

function isJsonArray(value: JsonValue): value is JsonArray {
  return Array.isArray(value);
}

function isJsonObject(value: JsonValue): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function canonicalize(value: JsonValue): string {
  if (value === null || typeof value === "boolean") return String(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new CanonicalError("invalid_number");
    const encoded = JSON.stringify(Object.is(value, -0) ? 0 : value);
    return encoded;
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (isJsonArray(value)) return `[${value.map(canonicalize).join(",")}]`;
  if (isJsonObject(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalize(value[key] ?? null)}`)
      .join(",")}}`;
  }
  throw new CanonicalError("serialization_failed");
}
