import type { JsonValue } from "../canonical.js";
import type { ParsedDirective } from "../directives/types.js";
import type { IslandRecord } from "../islands/record.js";
import { createPromotionNonce } from "../islands/nonce.js";
import type { RuntimeRandomness } from "../runtime/ports.js";

export interface IntentSource {
  readonly island: IslandRecord;
  readonly element: Element;
  readonly directive: ParsedDirective;
  readonly eventType: string;
  readonly trusted: boolean;
}

export type ServerOperation =
  | Readonly<{ kind: "sync_model"; field: string }>
  | Readonly<{
      kind: "invoke_action";
      name: string;
      arguments: Readonly<Record<string, JsonValue>>;
    }>
  | Readonly<{ kind: "params_changed" }>
  | Readonly<{ kind: "lazy_complete" }>
  | Readonly<{ kind: "fresh_render" }>;

export type IntentFinishReason = "accepted" | "terminal" | "canceled" | "exhausted" | "rejected";

const MAX_OPERATIONS_PER_INTENT = 32;
const MAX_INTENT_JSON_DEPTH = 32;
const MAX_INTENT_JSON_NODES = 2_048;

function immutableJson(value: JsonValue, depth: number, budget: { remaining: number }): JsonValue {
  if (depth > MAX_INTENT_JSON_DEPTH || budget.remaining <= 0) throw new Error("intent_json_limit");
  budget.remaining -= 1;
  if (Array.isArray(value)) {
    return Object.freeze(
      value.map((item) => immutableJson(item as JsonValue, depth + 1, budget)),
    );
  }
  if (value !== null && typeof value === "object") {
    const result: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
    for (const [key, item] of Object.entries(value)) {
      result[key] = immutableJson(item, depth + 1, budget);
    }
    return Object.freeze(result);
  }
  return value;
}

function immutableOperation(operation: ServerOperation): ServerOperation {
  if (operation.kind !== "invoke_action") return Object.freeze({ ...operation });
  const budget = { remaining: MAX_INTENT_JSON_NODES };
  return Object.freeze({
    ...operation,
    arguments: immutableJson(operation.arguments, 0, budget) as Readonly<Record<string, JsonValue>>,
  });
}

export class ServerIntent {
  readonly source: IntentSource;
  readonly operations: readonly ServerOperation[];
  #nonce: string | null;
  #finished = false;
  readonly #finishCallbacks: VoidFunction[] = [];

  constructor(source: IntentSource, operations: readonly ServerOperation[], nonce: string | null) {
    if (operations.length === 0 || operations.length > MAX_OPERATIONS_PER_INTENT) {
      throw new Error("intent_operation_limit");
    }
    this.source = Object.freeze(source);
    this.operations = Object.freeze(operations.map(immutableOperation));
    this.#nonce = nonce;
    Object.freeze(this);
  }

  promotionNonce(): string | null {
    return this.#nonce;
  }

  onFinish(callback: VoidFunction): void {
    if (this.#finished) {
      callback();
      return;
    }
    this.#finishCallbacks.push(callback);
  }

  finish(reason: IntentFinishReason): void {
    void reason;
    if (this.#finished) return;
    this.#finished = true;
    this.#nonce = null;
    for (const callback of this.#finishCallbacks.splice(0)) callback();
  }
}

export function createServerIntent(
  source: IntentSource,
  operations: readonly ServerOperation[],
  randomness: RuntimeRandomness,
  promotion: boolean,
): ServerIntent {
  const nonce = promotion ? createPromotionNonce(randomness) : null;
  return new ServerIntent(source, operations, nonce);
}
