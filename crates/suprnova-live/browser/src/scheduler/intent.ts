import type { JsonValue } from "../canonical.js";
import type { ParsedDirective } from "../directives/types.js";
import type { IslandRecord } from "../islands/record.js";
import { createPromotionNonce } from "../islands/nonce.js";
import type { RuntimeRandomness } from "../runtime/ports.js";
import type { FreshRenderReason } from "../features/contract.js";

export interface IntentSource {
  readonly island: IslandRecord;
  readonly element: Element;
  readonly directive: ParsedDirective | null;
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

export type IntentFinishReason =
  "accepted" | "terminal" | "canceled" | "superseded" | "retired" | "exhausted" | "rejected";
export type IntentFinishObserver = (reason: IntentFinishReason) => void;

const MAX_OPERATIONS_PER_INTENT = 32;
const MAX_MODEL_PROPOSALS_PER_INTENT = 128;
const MAX_INTENT_JSON_DEPTH = 32;
const MAX_INTENT_JSON_NODES = 2_048;
const MAX_FINISH_CALLBACKS = 64;
const MODEL_FIELD = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/u;

function immutableJson(value: JsonValue, depth: number, budget: { remaining: number }): JsonValue {
  if (depth > MAX_INTENT_JSON_DEPTH || budget.remaining <= 0) throw new Error("intent_json_limit");
  budget.remaining -= 1;
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => immutableJson(item as JsonValue, depth + 1, budget)));
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
  readonly modelProposals: Readonly<Record<string, JsonValue>>;
  readonly modelEditSequences: Readonly<Record<string, bigint>>;
  readonly childParameters: Readonly<Record<string, JsonValue>> | undefined;
  #nonce: string | null;
  #finishReason: IntentFinishReason | null = null;
  readonly #finishCallbacks: IntentFinishObserver[] = [];

  constructor(
    source: IntentSource,
    operations: readonly ServerOperation[],
    nonce: string | null,
    modelProposals: Readonly<Record<string, JsonValue>> = Object.freeze({}),
    modelEditSequences: Readonly<Record<string, bigint>> = Object.freeze({}),
    childParameters?: Readonly<Record<string, JsonValue>>,
  ) {
    if (operations.length === 0 || operations.length > MAX_OPERATIONS_PER_INTENT) {
      throw new Error("intent_operation_limit");
    }
    this.source = Object.freeze(source);
    this.operations = Object.freeze(operations.map(immutableOperation));
    const proposalEntries = Object.entries(modelProposals);
    const sequenceEntries = Object.entries(modelEditSequences);
    if (
      proposalEntries.length > MAX_MODEL_PROPOSALS_PER_INTENT ||
      sequenceEntries.length !== proposalEntries.length
    ) {
      throw new Error("intent_model_proposal_limit");
    }
    const synchronizedOperations = this.operations.filter(
      (operation) => operation.kind === "sync_model",
    );
    const synchronized = new Set(synchronizedOperations.map((operation) => operation.field));
    const proposals: Record<string, JsonValue> = {};
    const sequences: Record<string, bigint> = {};
    const proposalBudget = { remaining: MAX_INTENT_JSON_NODES };
    for (const [field, value] of proposalEntries) {
      const sequence = modelEditSequences[field];
      if (
        !MODEL_FIELD.test(field) ||
        !synchronized.has(field) ||
        typeof sequence !== "bigint" ||
        sequence < 0n
      ) {
        throw new Error("intent_model_proposal_invalid");
      }
      proposals[field] = immutableJson(value, 0, proposalBudget);
      sequences[field] = sequence;
    }
    if (
      synchronizedOperations.length !== synchronized.size ||
      synchronized.size !== proposalEntries.length ||
      sequenceEntries.some(([field]) => !Object.prototype.hasOwnProperty.call(proposals, field))
    ) {
      throw new Error("intent_model_proposal_invalid");
    }
    this.modelProposals = Object.freeze(proposals);
    this.modelEditSequences = Object.freeze(sequences);
    this.childParameters =
      childParameters === undefined
        ? undefined
        : (immutableJson(childParameters, 0, {
            remaining: MAX_INTENT_JSON_NODES,
          }) as Readonly<Record<string, JsonValue>>);
    this.#nonce = nonce;
    Object.freeze(this);
  }

  promotionNonce(): string | null {
    return this.#nonce;
  }

  onFinish(callback: IntentFinishObserver): void {
    if (this.#finishReason !== null) {
      try {
        callback(this.#finishReason);
      } catch {
        // Completion observers cannot change the already-terminal intent.
      }
      return;
    }
    if (this.#finishCallbacks.length >= MAX_FINISH_CALLBACKS) {
      throw new Error("intent_finish_callback_limit");
    }
    this.#finishCallbacks.push(callback);
  }

  finish(reason: IntentFinishReason): void {
    if (this.#finishReason !== null) return;
    this.#finishReason = reason;
    this.#nonce = null;
    for (const callback of this.#finishCallbacks.splice(0)) {
      try {
        callback(reason);
      } catch {
        // One observer cannot prevent the remaining bounded cleanup callbacks.
      }
    }
  }
}

export function createParamsChangedIntent(
  island: IslandRecord,
  envelope: Readonly<Record<string, JsonValue>>,
  parentSnapshot: Readonly<Record<string, JsonValue>>,
): ServerIntent {
  return new ServerIntent(
    Object.freeze({
      directive: null,
      element: island.element,
      eventType: "params_changed",
      island,
      trusted: false,
    }),
    Object.freeze([{ kind: "params_changed" }]),
    null,
    Object.freeze({}),
    Object.freeze({}),
    Object.freeze({ envelope, parent_snapshot: parentSnapshot }),
  );
}

export function createFreshRenderIntent(
  island: IslandRecord,
  reason?: FreshRenderReason,
): ServerIntent {
  return new ServerIntent(
    Object.freeze({
      directive: null,
      element: island.element,
      eventType: reason ?? "fresh_render",
      island,
      trusted: false,
    }),
    Object.freeze([{ kind: "fresh_render" }]),
    null,
  );
}

export function createServerIntent(
  source: IntentSource,
  operations: readonly ServerOperation[],
  randomness: RuntimeRandomness,
  promotion: boolean,
  modelProposals: Readonly<Record<string, JsonValue>> = Object.freeze({}),
  modelEditSequences: Readonly<Record<string, bigint>> = Object.freeze({}),
): ServerIntent {
  const nonce = promotion ? createPromotionNonce(randomness) : null;
  return new ServerIntent(source, operations, nonce, modelProposals, modelEditSequences);
}
