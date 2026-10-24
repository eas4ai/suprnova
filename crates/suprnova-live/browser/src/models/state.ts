import type { JsonValue } from "../canonical.js";
import {
  MISSING,
  immutableModelValue,
  isMissing,
  modelValuesEqual,
  type Missing,
  type ModelValue,
} from "./value.js";

const MODEL_FIELD = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/u;
const VALIDATION_MESSAGE = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
// Fields and validation issues are what the server rendered and returned, so
// they have no cap here; observers are runtime-internal subscribers.
const MAX_MODEL_OBSERVERS = 8;

export interface ValidationIssue {
  /// The stable message identity the server named (`validation.length`).
  readonly message: string;
  /// The text the server resolved for the application user, when it had one.
  readonly text?: string;
}

/// The validation entries of one field as the response carries them: a
/// message identity, an object naming the identity and its resolved text, or
/// a list of either. Anything else is not an issue and is dropped; an entry
/// whose text is not a non-empty string keeps its identity alone.
export function validationIssuesFrom(raw: unknown): readonly ValidationIssue[] {
  const entries = Array.isArray(raw) ? raw : [raw];
  const issues: ValidationIssue[] = [];
  for (const entry of entries) {
    if (typeof entry === "string") {
      if (VALIDATION_MESSAGE.test(entry)) issues.push(Object.freeze({ message: entry }));
      continue;
    }
    if (typeof entry !== "object" || entry === null || Array.isArray(entry)) continue;
    const record = entry as Readonly<Record<string, unknown>>;
    const message = record["id"];
    if (typeof message !== "string" || !VALIDATION_MESSAGE.test(message)) continue;
    const text = record["text"];
    issues.push(
      typeof text === "string" && text.length > 0
        ? Object.freeze({ message, text })
        : Object.freeze({ message }),
    );
  }
  return Object.freeze(issues);
}

function validationIssuesEqual(
  left: readonly ValidationIssue[],
  right: readonly ValidationIssue[],
): boolean {
  return (
    left.length === right.length &&
    left.every((issue, index) => {
      const other = right[index];
      if (other === undefined) return false;
      return issue.message === other.message && issue.text === other.text;
    })
  );
}

export interface ModelFieldState {
  readonly field: string;
  readonly browserProposal: JsonValue | Missing;
  readonly acceptedServerValue: JsonValue | Missing;
  readonly validation: readonly ValidationIssue[];
  readonly inFlightIntent: string | null;
  readonly editSequence: bigint;
}

export interface ModelEditResult {
  readonly changed: boolean;
  readonly editSequence: bigint;
}

interface MutableFieldState {
  readonly field: string;
  browserProposal: ModelValue;
  acceptedServerValue: ModelValue;
  validation: readonly ValidationIssue[];
  inFlightIntent: string | null;
  editSequence: bigint;
}

export class ModelState {
  readonly #fields = new Map<string, MutableFieldState>();
  readonly #observers = new Set<VoidFunction>();

  subscribe(observer: VoidFunction): VoidFunction {
    if (this.#observers.size >= MAX_MODEL_OBSERVERS) throw new Error("model_observer_limit");
    this.#observers.add(observer);
    let subscribed = true;
    return () => {
      if (!subscribed) return;
      subscribed = false;
      this.#observers.delete(observer);
    };
  }

  register(field: string, accepted: ModelValue = MISSING): ModelFieldState {
    validateField(field);
    const existing = this.#fields.get(field);
    if (existing !== undefined) {
      if (isMissing(existing.acceptedServerValue) && !isMissing(accepted)) {
        const value = immutableModelValue(accepted);
        existing.acceptedServerValue = value;
        existing.browserProposal = value;
        this.#notify();
      }
      return snapshot(existing);
    }
    const initial = isMissing(accepted) ? MISSING : immutableModelValue(accepted);
    const state: MutableFieldState = {
      acceptedServerValue: initial,
      browserProposal: initial,
      editSequence: 0n,
      field,
      inFlightIntent: null,
      validation: Object.freeze([]),
    };
    this.#fields.set(field, state);
    this.#notify();
    return snapshot(state);
  }

  propose(field: string, value: ModelValue): ModelEditResult {
    const state = this.#required(field);
    const proposal = isMissing(value) ? MISSING : immutableModelValue(value);
    if (modelValuesEqual(state.browserProposal, proposal)) {
      return Object.freeze({ changed: false, editSequence: state.editSequence });
    }
    state.browserProposal = proposal;
    state.editSequence += 1n;
    this.#notify();
    return Object.freeze({ changed: true, editSequence: state.editSequence });
  }

  reset(field: string, value: ModelValue): ModelEditResult {
    return this.propose(field, value);
  }

  dirty(field: string): boolean {
    const state = this.#required(field);
    return !modelValuesEqual(state.browserProposal, state.acceptedServerValue);
  }

  setValidation(field: string, issues: readonly ValidationIssue[]): void {
    const state = this.#required(field);
    const normalized = normalizeValidation(issues);
    if (validationIssuesEqual(state.validation, normalized)) return;
    state.validation = normalized;
    this.#notify();
  }

  markInFlight(field: string, intent: string): void {
    if (!VALIDATION_MESSAGE.test(intent)) throw new Error("model_intent_invalid");
    const state = this.#required(field);
    if (state.inFlightIntent === intent) return;
    state.inFlightIntent = intent;
    this.#notify();
  }

  clearInFlight(field: string, intent?: string): void {
    const state = this.#required(field);
    if (
      (intent === undefined || state.inFlightIntent === intent) &&
      state.inFlightIntent !== null
    ) {
      state.inFlightIntent = null;
      this.#notify();
    }
  }

  reconcile(
    field: string,
    accepted: ModelValue,
    submittedEditSequence: bigint,
    validation: readonly ValidationIssue[],
    intent?: string,
  ): boolean {
    const state = this.#required(field);
    if (submittedEditSequence < 0n || submittedEditSequence > state.editSequence) return false;
    const value = isMissing(accepted) ? MISSING : immutableModelValue(accepted);
    const normalized = normalizeValidation(validation);
    const acceptedChanged = !modelValuesEqual(state.acceptedServerValue, value);
    const proposalChanged =
      submittedEditSequence === state.editSequence &&
      !modelValuesEqual(state.browserProposal, value);
    const validationChanged = !validationIssuesEqual(state.validation, normalized);
    const inFlightChanged =
      state.inFlightIntent !== null && (intent === undefined || state.inFlightIntent === intent);
    state.acceptedServerValue = value;
    state.validation = normalized;
    if (submittedEditSequence === state.editSequence) state.browserProposal = value;
    if (intent === undefined || state.inFlightIntent === intent) state.inFlightIntent = null;
    if (acceptedChanged || proposalChanged || validationChanged || inFlightChanged) this.#notify();
    return true;
  }

  /**
   * Takes an applied render into the field without counting an edit: the
   * value the render gave its controls is the accepted server value, and the
   * value the controls show once local edits are restored is the baseline the
   * next edit is compared with (LIVE-037).
   */
  settle(field: string, accepted: ModelValue, shown: ModelValue): void {
    const state = this.#required(field);
    const acceptedValue = isMissing(accepted) ? MISSING : immutableModelValue(accepted);
    const shownValue = isMissing(shown) ? MISSING : immutableModelValue(shown);
    const changed =
      !modelValuesEqual(state.acceptedServerValue, acceptedValue) ||
      !modelValuesEqual(state.browserProposal, shownValue);
    state.acceptedServerValue = acceptedValue;
    state.browserProposal = shownValue;
    if (changed) this.#notify();
  }

  proposal(field: string): ModelValue {
    return this.#required(field).browserProposal;
  }

  editSequence(field: string): bigint {
    return this.#required(field).editSequence;
  }

  snapshot(field: string): ModelFieldState {
    return snapshot(this.#required(field));
  }

  fields(): readonly string[] {
    return Object.freeze([...this.#fields.keys()].sort());
  }

  #required(field: string): MutableFieldState {
    const state = this.#fields.get(field);
    if (state === undefined) throw new Error("model_field_missing");
    return state;
  }

  #notify(): void {
    for (const observer of this.#observers) {
      try {
        observer();
      } catch {
        // Presentation observers cannot change model authority.
      }
    }
  }
}

function validateField(field: string): void {
  if (!MODEL_FIELD.test(field)) throw new Error("model_field_invalid");
}

function normalizeValidation(issues: readonly ValidationIssue[]): readonly ValidationIssue[] {
  return Object.freeze(
    issues.map((issue) => {
      if (!VALIDATION_MESSAGE.test(issue.message)) throw new Error("model_validation_invalid");
      if (issue.text !== undefined && (typeof issue.text !== "string" || issue.text.length === 0)) {
        throw new Error("model_validation_invalid");
      }
      return issue.text === undefined
        ? Object.freeze({ message: issue.message })
        : Object.freeze({ message: issue.message, text: issue.text });
    }),
  );
}

function snapshot(state: MutableFieldState): ModelFieldState {
  return Object.freeze({
    acceptedServerValue: state.acceptedServerValue,
    browserProposal: state.browserProposal,
    editSequence: state.editSequence,
    field: state.field,
    inFlightIntent: state.inFlightIntent,
    validation: state.validation,
  });
}
