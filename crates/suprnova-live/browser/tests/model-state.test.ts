import fc from "fast-check";
import { describe, expect, it } from "vitest";

import type { JsonValue } from "../src/canonical.js";
import {
  controlEligibleForModel,
  readModelControl,
  type ModelControlRead,
} from "../src/models/control.js";
import { buildModelBatch } from "../src/models/forms.js";
import { ModelState } from "../src/models/state.js";
import { MISSING, modelValuesEqual } from "../src/models/value.js";
import type { IntentSource } from "../src/scheduler/intent.js";
import { ServerIntent } from "../src/scheduler/intent.js";

function input(overrides: Readonly<Record<string, unknown>>): HTMLInputElement {
  return {
    checked: false,
    disabled: false,
    tagName: "INPUT",
    type: "text",
    value: "",
    ...overrides,
  } as unknown as HTMLInputElement;
}

function select(overrides: Readonly<Record<string, unknown>>): HTMLSelectElement {
  return {
    disabled: false,
    multiple: false,
    options: [],
    selectedIndex: -1,
    tagName: "SELECT",
    value: "",
    ...overrides,
  } as unknown as HTMLSelectElement;
}

function value(read: ModelControlRead): JsonValue {
  expect(read.kind).toBe("value");
  if (read.kind !== "value") throw new Error("expected model control value");
  return read.value;
}

describe("model control values", () => {
  it("keeps missing distinct from explicit null and compares canonical JSON structurally", () => {
    expect(MISSING).not.toBeNull();
    expect(modelValuesEqual(MISSING, MISSING)).toBe(true);
    expect(modelValuesEqual(MISSING, null)).toBe(false);
    expect(modelValuesEqual({ b: 2, a: 1 }, { a: 1, b: 2 })).toBe(true);
  });

  it("maps text, number, checkbox, radio, select, and multi-select semantics", () => {
    expect(value(readModelControl(input({ value: "Ada" })))).toBe("Ada");
    expect(value(readModelControl(input({ type: "number", value: "42.5" })))).toBe(42.5);
    expect(value(readModelControl(input({ type: "number", value: "" })))).toBeNull();
    expect(value(readModelControl(input({ checked: true, type: "checkbox" })))).toBe(true);
    expect(value(readModelControl(input({ checked: false, type: "checkbox" })))).toBe(false);
    expect(value(readModelControl(input({ checked: true, type: "radio", value: "pro" })))).toBe(
      "pro",
    );
    expect(readModelControl(input({ checked: false, type: "radio", value: "pro" }))).toEqual({
      kind: "missing",
    });
    expect(value(readModelControl(select({ selectedIndex: 0, value: "rust" })))).toBe("rust");
    expect(value(readModelControl(select({ selectedIndex: -1 })))).toBeNull();
    expect(
      value(
        readModelControl(
          select({
            multiple: true,
            options: [
              { selected: true, value: "rust" },
              { selected: false, value: "go" },
              { selected: true, value: "zig" },
            ],
          }),
        ),
      ),
    ).toEqual(["rust", "zig"]);
  });

  it("fails closed on malformed controls and excludes disabled and file controls", () => {
    expect(readModelControl(input({ type: "file" }))).toEqual({ kind: "unsupported_file" });
    expect(readModelControl(input({ type: "number", value: "not-a-number" }))).toEqual({
      code: "number_invalid",
      kind: "invalid",
    });
    expect(readModelControl({ disabled: false, tagName: "BUTTON" } as unknown as Element)).toEqual({
      code: "control_unsupported",
      kind: "invalid",
    });
    expect(controlEligibleForModel(input({ disabled: true }))).toBe(false);
    expect(controlEligibleForModel(input({ disabled: false }))).toBe(true);
  });
});

describe("per-island model state", () => {
  it("keeps proposal, accepted authority, validation, and in-flight identity separate", () => {
    const state = new ModelState();
    state.register("profile.name", "Ada");
    const proposed = state.propose("profile.name", "Grace");
    state.setValidation("profile.name", [{ message: "validation.required" }]);
    state.markInFlight("profile.name", "intent-1");

    expect(proposed).toEqual({ changed: true, editSequence: 1n });
    expect(state.dirty("profile.name")).toBe(true);
    expect(state.snapshot("profile.name")).toEqual({
      acceptedServerValue: "Ada",
      browserProposal: "Grace",
      editSequence: 1n,
      field: "profile.name",
      inFlightIntent: "intent-1",
      validation: [{ message: "validation.required" }],
    });
  });

  it("advances accepted authority without overwriting a newer browser edit", () => {
    const state = new ModelState();
    state.register("query", "old");
    const submitted = state.propose("query", "first").editSequence;
    state.markInFlight("query", "intent-1");
    state.propose("query", "newer");

    expect(state.reconcile("query", "FIRST", submitted, [])).toBe(true);
    expect(state.snapshot("query")).toMatchObject({
      acceptedServerValue: "FIRST",
      browserProposal: "newer",
      editSequence: 2n,
      inFlightIntent: null,
      validation: [],
    });
    expect(state.dirty("query")).toBe(true);
  });

  it("adopts a normalized accepted value when no newer edit exists and resets predictably", () => {
    const state = new ModelState();
    state.register("quantity", null);
    const submitted = state.propose("quantity", 4).editSequence;
    expect(state.reconcile("quantity", 5, submitted, [])).toBe(true);
    expect(state.snapshot("quantity")).toMatchObject({
      acceptedServerValue: 5,
      browserProposal: 5,
    });
    expect(state.dirty("quantity")).toBe(false);

    expect(state.reset("quantity", null)).toEqual({ changed: true, editSequence: 2n });
    expect(state.snapshot("quantity").browserProposal).toBeNull();
  });

  it("builds one ordered proposal batch while excluding missing, disabled, and file values", () => {
    const batch = buildModelBatch([
      { editSequence: 4n, eligible: true, field: "zeta", read: { kind: "value", value: null } },
      { editSequence: 2n, eligible: false, field: "disabled", read: { kind: "value", value: 2 } },
      { editSequence: 3n, eligible: true, field: "alpha", read: { kind: "value", value: "a" } },
      { editSequence: 1n, eligible: true, field: "radio", read: { kind: "missing" } },
      { editSequence: 1n, eligible: true, field: "upload", read: { kind: "unsupported_file" } },
    ]);

    expect(batch.operations).toEqual([
      { field: "alpha", kind: "sync_model" },
      { field: "zeta", kind: "sync_model" },
    ]);
    expect(batch.proposals).toEqual({ alpha: "a", zeta: null });
    expect(batch.editSequences).toEqual({ alpha: 3n, zeta: 4n });
  });

  it("attaches immutable proposal values and browser edit sequences beside wire operations", () => {
    const proposals: Record<string, JsonValue> = { query: "rust" };
    const sequences: Record<string, bigint> = { query: 7n };
    const intent = new ServerIntent(
      Object.freeze({ eventType: "input" }) as unknown as IntentSource,
      [Object.freeze({ field: "query", kind: "sync_model" })],
      null,
      proposals,
      sequences,
    );
    proposals["query"] = "mutated";
    sequences["query"] = 9n;

    expect(intent.modelProposals).toEqual({ query: "rust" });
    expect(intent.modelEditSequences).toEqual({ query: 7n });
    expect(Object.isFrozen(intent.modelProposals)).toBe(true);
    expect(Object.isFrozen(intent.modelEditSequences)).toBe(true);
  });

  it("rejects missing, extra, or duplicate model proposal authority", () => {
    const source = Object.freeze({ eventType: "input" }) as unknown as IntentSource;
    expect(
      () => new ServerIntent(source, [Object.freeze({ field: "query", kind: "sync_model" })], null),
    ).toThrow("intent_model_proposal_invalid");
    expect(
      () =>
        new ServerIntent(
          source,
          [
            Object.freeze({ field: "query", kind: "sync_model" }),
            Object.freeze({ field: "query", kind: "sync_model" }),
          ],
          null,
          { query: "rust" },
          { query: 1n },
        ),
    ).toThrow("intent_model_proposal_invalid");
  });

  it("shares one bounded JSON-node budget across the complete proposal batch", () => {
    const source = Object.freeze({ eventType: "submit" }) as unknown as IntentSource;
    const large = Array.from({ length: 1_100 }, () => 1);
    expect(
      () =>
        new ServerIntent(
          source,
          [
            Object.freeze({ field: "first", kind: "sync_model" }),
            Object.freeze({ field: "second", kind: "sync_model" }),
          ],
          null,
          { first: large, second: large },
          { first: 1n, second: 1n },
        ),
    ).toThrow("intent_json_limit");
  });

  it("never replaces a proposal newer than the accepted response edit sequence", () => {
    fc.assert(
      fc.property(
        fc.array(fc.string({ maxLength: 24 }), { maxLength: 32, minLength: 1 }),
        (edits) => {
          const state = new ModelState();
          state.register("query", "accepted-0");
          const sequences = edits.map((edit) => state.propose("query", edit).editSequence);
          const responseIndex = Math.floor(sequences.length / 2);
          const responseSequence = sequences[responseIndex] ?? 0n;
          const latest = state.snapshot("query").browserProposal;

          expect(state.reconcile("query", "server-normalized", responseSequence, [])).toBe(true);
          expect(state.snapshot("query").acceptedServerValue).toBe("server-normalized");
          if (responseSequence < state.snapshot("query").editSequence) {
            expect(state.snapshot("query").browserProposal).toEqual(latest);
          }
        },
      ),
      { numRuns: 200 },
    );
  });
});
