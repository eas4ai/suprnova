import type { ModelState } from "../models/state.js";
import type { IntentDisposition } from "../scheduler/types.js";

export type FeedbackState =
  | "idle"
  | "dirty"
  | "queued"
  | "loading"
  | "validating"
  | "success"
  | "interrupted"
  | "offline"
  | "retrying"
  | "error";

export interface FeedbackSnapshot {
  readonly states: ReadonlySet<FeedbackState>;
  readonly intentId: string | null;
  readonly field: string | null;
  readonly action: string | null;
}

export type FeedbackScope =
  | { readonly kind: "island"; readonly value: string }
  | { readonly kind: "field"; readonly value: string }
  | { readonly kind: "action"; readonly value: string };

export type FeedbackWorkPhase =
  "pending" | "in_flight" | "response_ready" | "applying" | "completed";

export interface FeedbackWorkRecord {
  readonly intentId: string;
  readonly fields: readonly string[];
  readonly actions: readonly string[];
  readonly phase: FeedbackWorkPhase;
  readonly disposition: IntentDisposition | null;
  readonly retrying: boolean;
  readonly offline: boolean;
}

function relevant(record: FeedbackWorkRecord, scope: FeedbackScope): boolean {
  if (scope.kind === "island") return true;
  return scope.kind === "field"
    ? record.fields.includes(scope.value)
    : record.actions.includes(scope.value);
}

function terminalState(disposition: IntentDisposition | null): FeedbackState | null {
  switch (disposition) {
    case "accepted":
      return "success";
    case "canceled":
    case "superseded":
    case "retired":
      return "interrupted";
    case "rejected":
    case "duplicate":
    case "stale":
    case "out_of_order":
    case "incompatible":
      return "error";
    case null:
      return null;
  }
}

function addWorkStates(
  states: Set<FeedbackState>,
  record: FeedbackWorkRecord,
  scope: FeedbackScope,
): void {
  if (record.phase === "pending") states.add("queued");
  if (record.phase === "in_flight" || record.phase === "response_ready") {
    states.add("loading");
  }
  if (
    record.phase === "applying" ||
    (record.phase === "in_flight" && scope.kind === "field" && record.fields.includes(scope.value))
  ) {
    states.add("validating");
  }
  if (record.retrying) states.add("retrying");
  if (record.offline) states.add("offline");
  if (record.phase === "completed") {
    const terminal = terminalState(record.disposition);
    if (terminal !== null) states.add(terminal);
  }
}

function addModelStates(
  states: Set<FeedbackState>,
  model: ModelState | null,
  scope: FeedbackScope,
) {
  if (model === null || scope.kind !== "field" || !model.fields().includes(scope.value)) return;
  const snapshot = model.snapshot(scope.value);
  if (model.dirty(scope.value)) states.add("dirty");
  if (snapshot.validation.length > 0) states.add("error");
}

export function projectFeedback(
  records: readonly FeedbackWorkRecord[],
  model: ModelState | null,
  scope: FeedbackScope,
): FeedbackSnapshot {
  const states = new Set<FeedbackState>();
  const matching = records.filter((record) => relevant(record, scope));
  for (const record of matching) addWorkStates(states, record, scope);
  addModelStates(states, model, scope);
  if (states.size === 0) states.add("idle");
  const latest = matching[matching.length - 1] ?? null;
  return Object.freeze({
    action: scope.kind === "action" ? scope.value : null,
    field: scope.kind === "field" ? scope.value : null,
    intentId: latest?.intentId ?? null,
    states: new Set(states),
  });
}
