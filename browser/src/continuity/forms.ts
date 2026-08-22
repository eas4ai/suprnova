import type { MorphIdentityEntry, MorphPlan } from "../morph/types.js";
import {
  consumeContinuityBytes,
  ContinuityError,
  type ContinuityBudget,
  type ContinuityLimits,
  type ControlContinuity,
} from "./types.js";

const AUTHORITATIVE_ATTRIBUTE = "data-suprnova-live-authoritative";
const SAFE_SEQUENCE = /^(?:0|[1-9][0-9]{0,19})$/u;

function input(element: Element): element is HTMLInputElement {
  return element.tagName === "INPUT";
}

function textarea(element: Element): element is HTMLTextAreaElement {
  return element.tagName === "TEXTAREA";
}

function select(element: Element): element is HTMLSelectElement {
  return element.tagName === "SELECT";
}

function correction(entry: MorphIdentityEntry): boolean {
  const raw = entry.replacement?.getAttribute(AUTHORITATIVE_ATTRIBUTE);
  if (raw === null || raw === undefined) return false;
  if (!SAFE_SEQUENCE.test(raw)) throw new ContinuityError("invalid_authority");
  return true;
}

function selectedValues(element: HTMLSelectElement, defaults: boolean): readonly string[] {
  return Object.freeze(
    [...element.options]
      .filter((option) => (defaults ? option.defaultSelected : option.selected))
      .map((option) => option.value),
  );
}

function valuesEqual(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function captureEntry(
  plan: MorphPlan,
  entry: MorphIdentityEntry,
  budget: ContinuityBudget,
): ControlContinuity | null {
  const element = entry.current;
  if (element === null) return null;
  const authoritative = correction(entry);
  if (input(element)) {
    const type = element.type.toLowerCase();
    if (type === "file") {
      if ((element.files?.length ?? 0) === 0) return null;
      const control = plan.controls.byCurrent.get(element);
      if (entry.kind !== "live_key" || entry.replacement === null || control?.kind === "replace") {
        throw new ContinuityError("incompatible_state");
      }
      return Object.freeze({
        authoritative: false,
        element,
        identity: entry.token,
        kind: "file",
      });
    }
    if (type === "checkbox" || type === "radio") {
      if (element.checked === element.defaultChecked && !element.indeterminate) return null;
      return Object.freeze({
        authoritative,
        checked: element.checked,
        element,
        identity: entry.token,
        indeterminate: element.indeterminate,
        kind: "check",
      });
    }
    if (element.value === element.defaultValue) return null;
    consumeContinuityBytes(budget, element.value);
    return Object.freeze({
      authoritative,
      element,
      identity: entry.token,
      kind: "text",
      value: element.value,
    });
  }
  if (textarea(element)) {
    if (element.value === element.defaultValue) return null;
    consumeContinuityBytes(budget, element.value);
    return Object.freeze({
      authoritative,
      element,
      identity: entry.token,
      kind: "text",
      value: element.value,
    });
  }
  if (select(element)) {
    const values = selectedValues(element, false);
    if (valuesEqual(values, selectedValues(element, true))) return null;
    for (const value of values) consumeContinuityBytes(budget, value);
    return Object.freeze({
      authoritative,
      element,
      identity: entry.token,
      kind: "select",
      values,
    });
  }
  return null;
}

export function captureControls(
  plan: MorphPlan,
  limits: ContinuityLimits,
  budget: ContinuityBudget,
): readonly ControlContinuity[] {
  const records: ControlContinuity[] = [];
  for (const entry of plan.identity.entries) {
    const record = captureEntry(plan, entry, budget);
    if (record === null) continue;
    records.push(record);
    if (records.length > limits.maxControls) throw new ContinuityError("resource_exhausted");
  }
  return Object.freeze(records);
}

function owned(root: HTMLElement, element: Element): boolean {
  return element.isConnected && (element === root || root.contains(element));
}

export function restoreControls(root: HTMLElement, controls: readonly ControlContinuity[]): void {
  for (const control of controls) {
    if (!owned(root, control.element)) {
      if (control.kind === "file") throw new ContinuityError("incompatible_state");
      continue;
    }
    if (control.authoritative) continue;
    switch (control.kind) {
      case "check":
        control.element.checked = control.checked;
        control.element.indeterminate = control.indeterminate;
        break;
      case "file":
        break;
      case "select": {
        const expected = new Set(control.values);
        for (const option of control.element.options) {
          option.selected = expected.has(option.value);
        }
        if (!valuesEqual(selectedValues(control.element, false), control.values)) {
          throw new ContinuityError("incompatible_state");
        }
        break;
      }
      case "text":
        control.element.value = control.value;
        break;
    }
  }
}
