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
const MAX_MODEL_FIELDS = 512;
const MAX_VALIDATION_ISSUES = 64;

export interface ValidationIssue {
  readonly message: string;
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

  register(field: string, accepted: ModelValue = MISSING): ModelFieldState {
    validateField(field);
    const existing = this.#fields.get(field);
    if (existing !== undefined) {
      if (isMissing(existing.acceptedServerValue) && !isMissing(accepted)) {
        const value = immutableModelValue(accepted);
        existing.acceptedServerValue = value;
        existing.browserProposal = value;
      }
      return snapshot(existing);
    }
    if (this.#fields.size >= MAX_MODEL_FIELDS) throw new Error("model_field_limit");
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
    this.#required(field).validation = normalizeValidation(issues);
  }

  markInFlight(field: string, intent: string): void {
    if (!VALIDATION_MESSAGE.test(intent)) throw new Error("model_intent_invalid");
    this.#required(field).inFlightIntent = intent;
  }

  clearInFlight(field: string, intent?: string): void {
    const state = this.#required(field);
    if (intent === undefined || state.inFlightIntent === intent) state.inFlightIntent = null;
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
    state.acceptedServerValue = value;
    state.validation = normalizeValidation(validation);
    if (submittedEditSequence === state.editSequence) state.browserProposal = value;
    this.clearInFlight(field, intent);
    return true;
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
}

function validateField(field: string): void {
  if (!MODEL_FIELD.test(field)) throw new Error("model_field_invalid");
}

function normalizeValidation(issues: readonly ValidationIssue[]): readonly ValidationIssue[] {
  if (issues.length > MAX_VALIDATION_ISSUES) throw new Error("model_validation_limit");
  return Object.freeze(
    issues.map((issue) => {
      if (!VALIDATION_MESSAGE.test(issue.message)) throw new Error("model_validation_invalid");
      return Object.freeze({ message: issue.message });
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
