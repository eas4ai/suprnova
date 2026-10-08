import { describe, expect, it } from "vitest";

import type { RuntimeFeatureDirectiveOwnership } from "../src/features/contract.js";
import { captureUploadMorph, reconcileUploadMorph } from "../src/uploads/morph.js";

class KeyedElement {
  readonly #attributes = new Map<string, string>();

  constructor(
    readonly tagName: string,
    key: string | null,
    readonly type = "",
  ) {
    if (key !== null) this.#attributes.set("data-suprnova-live-key", key);
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }
}

function element(tagName: string, key: string | null, type = ""): Element {
  return new KeyedElement(tagName, key, type) as unknown as Element;
}

function ownership(
  name: "upload" | "progress",
  field: string,
  target: Element,
  role: "cancel" | "remove" | "retry" | null = null,
): RuntimeFeatureDirectiveOwnership {
  return {
    attributeName: `live:${name}${role === null ? "" : `.${role}`}`,
    directive: {
      capability: "uploads@1",
      modifiers: [],
      name,
      ok: true,
      role,
      value: field,
    },
    element: target,
  };
}

function surface() {
  const input = element("INPUT", "attachment-input", "file");
  const progress = element("DIV", "attachment-progress");
  const cancel = element("BUTTON", "attachment-cancel");
  const retry = element("BUTTON", "attachment-retry");
  const remove = element("BUTTON", "attachment-remove");
  return {
    cancel,
    input,
    ownership: [
      ownership("upload", "attachment", input),
      ownership("progress", "attachment", progress),
      ownership("upload", "attachment", cancel, "cancel"),
      ownership("upload", "attachment", retry, "retry"),
      ownership("upload", "attachment", remove, "remove"),
    ],
    progress,
    remove,
    retry,
  };
}

describe("upload keyed morph continuity", () => {
  it("preserves only the same keyed input, progress root, and controls", () => {
    const current = surface();
    const continuity = captureUploadMorph(current.ownership, ["attachment"]);

    expect(reconcileUploadMorph(continuity, current.ownership)).toEqual(["attachment"]);

    const replacedInput = element("INPUT", "attachment-input", "file");
    const replacement = current.ownership.map((entry) =>
      entry.element === current.input ? ownership("upload", "attachment", replacedInput) : entry,
    );
    expect(reconcileUploadMorph(continuity, replacement)).toEqual([]);
  });

  it("rejects rekeying, removal, and unkeyed active ownership", () => {
    const current = surface();
    const continuity = captureUploadMorph(current.ownership, ["attachment"]);
    current.input.setAttribute("data-suprnova-live-key", "attachment-input-rekeyed");
    expect(reconcileUploadMorph(continuity, current.ownership)).toEqual([]);

    const stable = surface();
    const stableContinuity = captureUploadMorph(stable.ownership, ["attachment"]);
    expect(
      reconcileUploadMorph(
        stableContinuity,
        stable.ownership.filter(({ element: target }) => target !== stable.progress),
      ),
    ).toEqual([]);

    const unkeyedInput = element("INPUT", null, "file");
    const unkeyed = [ownership("upload", "attachment", unkeyedInput)];
    expect(reconcileUploadMorph(captureUploadMorph(unkeyed, ["attachment"]), unkeyed)).toEqual([]);
  });

  it("does not make an unrelated field part of an active field continuity proof", () => {
    const current = surface();
    const avatarInput = element("INPUT", "avatar-input", "file");
    const withAvatar = [...current.ownership, ownership("upload", "avatar", avatarInput)];
    const continuity = captureUploadMorph(withAvatar, ["attachment"]);

    avatarInput.setAttribute("data-suprnova-live-key", "avatar-rekeyed");
    expect(reconcileUploadMorph(continuity, withAvatar)).toEqual(["attachment"]);
  });
});
