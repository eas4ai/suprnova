import { describe, expect, it, vi } from "vitest";

import { captureControls, restoreControls } from "../src/continuity/forms.js";
import { restoreFocus } from "../src/continuity/focus.js";
import { restoreContinuity } from "../src/continuity/restore.js";
import {
  consumeContinuityBytes,
  ContinuityError,
  type ContinuityRecord,
  type ControlContinuity,
  DEFAULT_CONTINUITY_LIMITS,
} from "../src/continuity/types.js";
import type { MorphPlan } from "../src/morph/types.js";

function rootContaining(...elements: Element[]): HTMLElement {
  return {
    contains: (candidate: Node | null) => elements.includes(candidate as Element),
  } as unknown as HTMLElement;
}

describe("interaction continuity", () => {
  it("bounds retained values by encoded bytes", () => {
    const budget = { bytes: 0, limit: 3 };
    consumeContinuityBytes(budget, "é");
    expect(budget.bytes).toBe(2);
    expect(() => {
      consumeContinuityBytes(budget, "é");
    }).toThrow(new ContinuityError("resource_exhausted"));
  });

  it("restores dirty text, check, and select presentation without emitting events", () => {
    const text = { isConnected: true, value: "server" } as unknown as HTMLInputElement;
    const check = {
      checked: false,
      indeterminate: false,
      isConnected: true,
    } as unknown as HTMLInputElement;
    const options = [
      { selected: true, value: "a" },
      { selected: false, value: "b" },
    ];
    const select = { isConnected: true, options } as unknown as HTMLSelectElement;
    const controls: readonly ControlContinuity[] = [
      { authoritative: false, element: text, identity: "text", kind: "text", value: "browser" },
      {
        authoritative: false,
        checked: true,
        element: check,
        identity: "check",
        indeterminate: true,
        kind: "check",
      },
      {
        authoritative: false,
        element: select,
        identity: "select",
        kind: "select",
        values: ["b"],
      },
    ];
    restoreControls(rootContaining(text, check, select), controls);
    expect(text.value).toBe("browser");
    expect({ checked: check.checked, indeterminate: check.indeterminate }).toEqual({
      checked: true,
      indeterminate: true,
    });
    expect(options.map(({ selected }) => selected)).toEqual([false, true]);
  });

  it("lets an explicit accepted correction win over a dirty browser value", () => {
    const text = { isConnected: true, value: "corrected" } as unknown as HTMLInputElement;
    restoreControls(rootContaining(text), [
      { authoritative: true, element: text, identity: "text", kind: "text", value: "browser" },
    ]);
    expect(text.value).toBe("corrected");
  });

  it("fails closed instead of fabricating a detached file input", () => {
    const file = { isConnected: false } as unknown as HTMLInputElement;
    expect(() => {
      restoreControls(rootContaining(), [
        { authoritative: false, element: file, identity: "file", kind: "file" },
      ]);
    }).toThrow(new ContinuityError("incompatible_state"));
  });

  it("does not fabricate continuity when a selected keyed file input is deliberately retired", () => {
    const file = {
      files: { length: 1 },
      getAttribute: () => null,
      tagName: "INPUT",
      type: "file",
    } as unknown as HTMLInputElement;
    const plan = {
      controls: { byCurrent: new Map<Element, never>() },
      identity: {
        entries: [
          {
            current: file,
            currentPosition: "root/0",
            kind: "live_key",
            replacement: null,
            replacementPosition: null,
            token: "live_key:attachment",
            value: "attachment",
          },
        ],
      },
    } as unknown as MorphPlan;

    expect(captureControls(plan, DEFAULT_CONTINUITY_LIMITS, { bytes: 0, limit: 1024 })).toEqual([]);

    const forcedReplacement = {
      ...plan,
      controls: { byCurrent: new Map([[file, { kind: "replace" }]]) },
      identity: {
        entries: [
          {
            ...plan.identity.entries[0],
            replacement: { getAttribute: () => null } as unknown as Element,
          },
        ],
      },
    } as unknown as MorphPlan;
    expect(
      captureControls(forcedReplacement, DEFAULT_CONTINUITY_LIMITS, {
        bytes: 0,
        limit: 1024,
      }),
    ).toEqual([]);
  });

  it("runs signal continuity inside the post-commit reconciliation phase", () => {
    const restoreSignals = vi.fn(() => 0);
    const record: ContinuityRecord = Object.freeze({
      composition: null,
      controls: Object.freeze([]),
      focusElement: null,
      focusedKey: null,
      focusVisible: false,
      scroll: Object.freeze([]),
      selections: Object.freeze([]),
      signalScopes: Object.freeze([]),
    });
    restoreContinuity(record, rootContaining(), { restoreSignals });
    expect(restoreSignals).toHaveBeenCalledOnce();
  });

  it("keeps the semantic island fallback programmatically focusable", () => {
    const setAttribute = vi.fn();
    const removeAttribute = vi.fn();
    const focus = vi.fn();
    const root = {
      closest: () => null,
      focus,
      getAttribute: () => null,
      hasAttribute: () => false,
      hidden: false,
      isConnected: true,
      querySelectorAll: () => [],
      removeAttribute,
      setAttribute,
    } as unknown as HTMLElement;

    restoreFocus(root, { element: null, focusedKey: "live_key:removed", focusVisible: true });

    expect(setAttribute).toHaveBeenCalledExactlyOnceWith("tabindex", "-1");
    expect(focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true });
    expect(removeAttribute).not.toHaveBeenCalled();
  });

  it("defers to a proven external identity that still owns document focus", () => {
    const rootFocus = vi.fn();
    const externalFocus = vi.fn();
    const documentAuthority = { activeElement: null as Element | null };
    const external = {
      closest: () => null,
      focus: externalFocus,
      getAttribute: () => null,
      hasAttribute: () => false,
      hidden: false,
      isConnected: true,
      ownerDocument: documentAuthority,
    } as unknown as HTMLElement;
    documentAuthority.activeElement = external;
    const root = {
      closest: () => null,
      contains: () => false,
      focus: rootFocus,
      getAttribute: () => null,
      hasAttribute: (name: string) => name === "tabindex",
      hidden: false,
      isConnected: true,
      querySelectorAll: () => [],
    } as unknown as HTMLElement;

    restoreFocus(root, {
      element: external,
      focusedKey: "id:teleported-focus",
      focusVisible: true,
    });

    expect(externalFocus).not.toHaveBeenCalled();
    expect(rootFocus).not.toHaveBeenCalled();
  });
});
