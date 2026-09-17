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
const number = (value: string) => control({ tagName: "INPUT", type: "number", value });
const checkbox = (checked: boolean, value = "on") =>
  control({ tagName: "INPUT", type: "checkbox", checked, value });
const radio = (checked: boolean, value: string) =>
  control({ tagName: "INPUT", type: "radio", checked, value });

describe("model binding groups on submit", () => {
  it("reads an empty number input as null instead of refusing it", () => {
    expect(readBindingGroup([binding(emptyNumber())])).toEqual({ kind: "value", value: null });
  });

  it("reads a select with nothing selected as null", () => {
    const select = control({ tagName: "SELECT", multiple: false, selectedIndex: -1, options: [] });
    expect(readBindingGroup([binding(select)])).toEqual({ kind: "value", value: null });
  });

  it("reads agreeing controls as one value and disagreeing ones as unsupported", () => {
    expect(readBindingGroup([binding(number("3")), binding(number("3"))])).toEqual({
      kind: "value",
      value: 3,
    });
    expect(readBindingGroup([binding(number("3")), binding(number("4"))])).toEqual({
      code: "control_unsupported",
      kind: "invalid",
    });
  });

  it("LIVE-032: reads a field more than one checkbox binds as the list of checked values", () => {
    const group = [
      binding(checkbox(true, "releases")),
      binding(checkbox(false, "security")),
      binding(checkbox(true, "events")),
    ];
    expect(readBindingGroup(group)).toEqual({ kind: "value", value: ["releases", "events"] });
    expect(
      readBindingGroup([
        binding(checkbox(false, "releases")),
        binding(checkbox(false, "security")),
      ]),
    ).toEqual({ kind: "value", value: [] });
  });

  it("LIVE-032: skips a disabled box of a group and keeps one checkbox a boolean", () => {
    const disabled = control({
      tagName: "INPUT",
      type: "checkbox",
      checked: true,
      value: "security",
      disabled: true,
    });
    expect(readBindingGroup([binding(checkbox(true, "releases")), binding(disabled)])).toEqual({
      kind: "value",
      value: ["releases"],
    });
    expect(readBindingGroup([binding(checkbox(true, "releases"))])).toEqual({
      kind: "value",
      value: true,
    });
  });

  it("LIVE-032: refuses a field that checkboxes and another control both bind", () => {
    expect(
      readBindingGroup([
        binding(checkbox(true, "releases")),
        binding(checkbox(false, "security")),
        binding(radio(true, "events")),
      ]),
    ).toEqual({ code: "control_unsupported", kind: "invalid" });
  });

  it("reads no eligible control as missing", () => {
    const disabled = control({ tagName: "INPUT", type: "number", value: "", disabled: true });
    expect(readBindingGroup([binding(disabled)])).toEqual({ kind: "missing" });
    expect(readBindingGroup([])).toEqual({ kind: "missing" });
  });
});
