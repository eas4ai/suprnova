import { describe, expect, it } from "vitest";

import { type ModelBinding, readBindingGroup } from "../src/models/forms.js";

// A form's submit samples every model binding associated with it, grouped by
// field. A control whose value is null, an empty number or range or a select
// with nothing selected, is a value like any other: the group must read it
// as null, not refuse the form as unsupported.

function control(shape: Readonly<Record<string, unknown>>): Element {
  return { disabled: false, matches: () => false, ...shape } as unknown as Element;
}

function binding(element: Element): ModelBinding {
  return { owned: { element } } as unknown as ModelBinding;
}

const emptyNumber = () => control({ tagName: "INPUT", type: "number", value: "" });
const checkbox = (checked: boolean) => control({ tagName: "INPUT", type: "checkbox", checked });

describe("model binding groups on submit", () => {
  it("reads an empty number input as null instead of refusing it", () => {
    expect(readBindingGroup([binding(emptyNumber())])).toEqual({ kind: "value", value: null });
  });

  it("reads a select with nothing selected as null", () => {
    const select = control({ tagName: "SELECT", multiple: false, selectedIndex: -1, options: [] });
    expect(readBindingGroup([binding(select)])).toEqual({ kind: "value", value: null });
  });

  it("reads agreeing controls as one value and disagreeing ones as unsupported", () => {
    expect(readBindingGroup([binding(checkbox(false)), binding(checkbox(false))])).toEqual({
      kind: "value",
      value: false,
    });
    expect(readBindingGroup([binding(checkbox(false)), binding(checkbox(true))])).toEqual({
      code: "control_unsupported",
      kind: "invalid",
    });
  });

  it("reads no eligible control as missing", () => {
    const disabled = control({ tagName: "INPUT", type: "number", value: "", disabled: true });
    expect(readBindingGroup([binding(disabled)])).toEqual({ kind: "missing" });
    expect(readBindingGroup([])).toEqual({ kind: "missing" });
  });
});
