import type { JsonValue } from "../canonical.js";
import { immutableModelValue } from "./value.js";

const MAX_SELECT_OPTIONS = 4_096;
const NON_VALUE_INPUT_TYPES = new Set(["button", "image", "reset", "submit"]);

export type ModelControlRead =
  | Readonly<{ kind: "value"; value: JsonValue }>
  | Readonly<{ kind: "missing" }>
  | Readonly<{ kind: "unsupported_file" }>
  | Readonly<{ kind: "invalid"; code: "control_unsupported" | "number_invalid" }>;

function disabledProperty(element: Element): boolean {
  return "disabled" in element && element.disabled === true;
}

export function controlEligibleForModel(element: Element): boolean {
  if (disabledProperty(element)) return false;
  try {
    if (typeof element.matches === "function" && element.matches(":disabled")) return false;
  } catch {
    return false;
  }
  return true;
}

export function readModelControl(element: Element): ModelControlRead {
  const tagName = element.tagName.toUpperCase();
  if (tagName === "TEXTAREA") {
    if (!("value" in element) || typeof element.value !== "string") return invalidControl();
    return modelValue(element.value);
  }
  if (tagName === "SELECT") return readSelect(element as HTMLSelectElement);
  if (tagName !== "INPUT") return invalidControl();
  return readInput(element as HTMLInputElement);
}

function readInput(element: HTMLInputElement): ModelControlRead {
  const type = element.type.toLowerCase();
  if (type === "file") return Object.freeze({ kind: "unsupported_file" });
  if (NON_VALUE_INPUT_TYPES.has(type)) return invalidControl();
  if (type === "checkbox") return modelValue(element.checked);
  if (type === "radio") {
    return element.checked ? modelValue(element.value) : Object.freeze({ kind: "missing" });
  }
  if (type === "number" || type === "range") {
    const source = element.value;
    if (source.length === 0) return modelValue(null);
    const number = Number(source);
    return Number.isFinite(number)
      ? modelValue(number)
      : Object.freeze({ code: "number_invalid", kind: "invalid" });
  }
  if (typeof element.value !== "string") return invalidControl();
  return modelValue(element.value);
}

function readSelect(element: HTMLSelectElement): ModelControlRead {
  if (!element.multiple) {
    return element.selectedIndex < 0 ? modelValue(null) : modelValue(element.value);
  }
  let options: HTMLOptionElement[];
  try {
    options = Array.from(element.options);
  } catch {
    return invalidControl();
  }
  if (options.length > MAX_SELECT_OPTIONS) return invalidControl();
  return modelValue(options.filter((option) => option.selected).map((option) => option.value));
}

function modelValue(value: JsonValue): ModelControlRead {
  return Object.freeze({ kind: "value", value: immutableModelValue(value) });
}

function invalidControl(): ModelControlRead {
  return Object.freeze({ code: "control_unsupported", kind: "invalid" });
}
