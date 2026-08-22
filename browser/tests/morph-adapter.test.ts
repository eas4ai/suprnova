import { beforeEach, describe, expect, it, vi } from "vitest";

import { consumeMorphProvenance, IdiomorphAdapter } from "../src/morph/idiomorph.js";
import { preflightIslandMorph } from "../src/morph/preflight.js";
import type { MorphPlan } from "../src/morph/types.js";
import { asElement, element, FakeDocument, morphFixture, withLimits } from "./support/morph-dom.js";

interface VendorCallbacks {
  beforeAttributeUpdated(
    name: string,
    node: Element,
    mutation: "update" | "remove",
  ): false | undefined;
  beforeNodeAdded(node: Node): false | undefined;
  beforeNodeMorphed(current: Node, replacement: Node): false | undefined;
  beforeNodeRemoved(node: Node): false | undefined;
}

interface VendorOptions {
  readonly callbacks: VendorCallbacks;
  readonly morphStyle: "outerHTML" | "innerHTML";
  readonly restoreFocus: boolean;
}

const morph = vi.hoisted(() =>
  vi.fn<(current: unknown, replacement: unknown, options: VendorOptions) => unknown>(),
);
vi.mock("idiomorph", () => ({ Idiomorph: { morph } }));

function plan(options: Parameters<typeof morphFixture>[0] = {}): MorphPlan {
  const fixture = morphFixture(options);
  return preflightIslandMorph({
    authority: fixture.authority,
    currentRoot: asElement(fixture.currentRoot),
    html: "<section></section>",
    limits: fixture.limits,
    parser: fixture.parser,
  });
}

beforeEach(() => {
  morph.mockReset();
});

describe("private Idiomorph adapter", () => {
  it("does not accept an unvalidated plan", () => {
    const adapter = new IdiomorphAdapter();
    expect(() => adapter.apply({} as never, {})).toThrow("morph_plan_invalid");
  });

  it("uses only outer-root Idiomorph mode and publishes deterministic identity results", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = [
      element(currentDocument, "li", { "data-suprnova-live-key": "alpha" }),
      element(currentDocument, "li", { "data-suprnova-live-key": "removed" }),
    ];
    const replacement = [
      element(replacementDocument, "li", { "data-suprnova-live-key": "inserted" }),
      element(replacementDocument, "li", { "data-suprnova-live-key": "alpha" }),
    ];
    const prepared = plan({ currentChildren: current, replacementChildren: replacement });
    morph.mockImplementation((_old, _new, options) => {
      options.callbacks.beforeNodeMorphed(prepared.currentRoot, prepared.replacementRoot);
      return [prepared.currentRoot];
    });

    const result = new IdiomorphAdapter(() => 0).apply(prepared, {});

    expect(morph).toHaveBeenCalledOnce();
    expect(morph.mock.calls[0]?.[2]).toMatchObject({
      morphStyle: "outerHTML",
      restoreFocus: false,
    });
    expect(
      morph.mock.calls[0]?.[2].callbacks.beforeAttributeUpdated(
        "data-suprnova-live-status",
        prepared.currentRoot,
        "remove",
      ),
    ).toBe(false);
    expect(result).toMatchObject({ inserted: ["inserted"], removed: ["removed"] });
    expect(result.moved).toContain("alpha");
    const moved = current[0];
    if (moved === undefined) throw new Error("missing moved fixture");
    expect(consumeMorphProvenance(asElement(moved))).toBe(true);
    expect(consumeMorphProvenance(asElement(moved))).toBe(false);
  });

  it("rejects unapproved insertion and adapter failure", () => {
    const prepared = plan();
    morph.mockImplementation((_old, _new, options) => {
      options.callbacks.beforeNodeAdded({ nodeType: 1 } as unknown as Node);
    });
    expect(() => new IdiomorphAdapter(() => 0).apply(prepared, {})).toThrow(
      "morph_unapproved_node",
    );

    morph.mockImplementation(() => {
      throw new Error("vendor_failed");
    });
    expect(() => new IdiomorphAdapter(() => 0).apply(prepared, {})).toThrow("vendor_failed");
  });

  it("assigns distinct private identities to removed and inserted Live keys", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const removed = element(currentDocument, "div", { "data-suprnova-live-key": "old" });
    const inserted = element(replacementDocument, "div", { "data-suprnova-live-key": "new" });
    const prepared = plan({ currentChildren: [removed], replacementChildren: [inserted] });
    morph.mockImplementation(() => {
      expect(removed.getAttribute("id")).toMatch(/^__suprnova_live_/u);
      expect(inserted.getAttribute("id")).toMatch(/^__suprnova_live_/u);
      expect(removed.getAttribute("id")).not.toBe(inserted.getAttribute("id"));
    });

    new IdiomorphAdapter(() => 0).apply(prepared, {});
    expect(removed.getAttribute("id")).toBeNull();
    expect(inserted.getAttribute("id")).toBeNull();
  });

  it("enforces non-disableable hook and deadline budgets", () => {
    const hookLimited = plan({ limits: withLimits({ maxHookCalls: 1 }) });
    morph.mockImplementation((_old, _new, options) => {
      options.callbacks.beforeNodeMorphed(hookLimited.currentRoot, hookLimited.replacementRoot);
    });
    expect(() => new IdiomorphAdapter(() => 0).apply(hookLimited, {})).toThrow("morph_hook_limit");

    const deadline = plan({ limits: withLimits({ deadlineMs: 1 }) });
    const now = vi.fn().mockReturnValueOnce(0).mockReturnValue(2);
    expect(() => new IdiomorphAdapter(now).apply(deadline, {})).toThrow("morph_deadline_exceeded");
    expect(morph).toHaveBeenCalledOnce();
  });

  it("preserves declared self attributes while continuing to morph descendants", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const currentChild = element(currentDocument, "span", { id: "preserve-child" });
    const replacementChild = element(replacementDocument, "span", { id: "preserve-child" });
    const current = element(
      currentDocument,
      "div",
      { "data-suprnova-live-key": "preserved", "live:preserve.self": "" },
      [currentChild],
    );
    const replacement = element(
      replacementDocument,
      "div",
      { "data-suprnova-live-key": "preserved", "live:preserve.self": "" },
      [replacementChild],
    );
    const prepared = plan({ currentChildren: [current], replacementChildren: [replacement] });
    morph.mockImplementation((_old, _new, options) => {
      expect(
        options.callbacks.beforeAttributeUpdated("data-state", asElement(current), "update"),
      ).toBe(false);
      expect(
        options.callbacks.beforeAttributeUpdated("data-state", asElement(currentChild), "update"),
      ).toBeUndefined();
      expect(
        options.callbacks.beforeNodeMorphed(asElement(currentChild), asElement(replacementChild)),
      ).toBeUndefined();
    });

    new IdiomorphAdapter(() => 0).apply(prepared, {});
  });

  it("keeps ignored descendants under the selected server or browser attribute policy", () => {
    for (const [modifier, rootSkipped] of [
      ["children", false],
      ["subtree", true],
    ] as const) {
      const currentDocument = new FakeDocument();
      const replacementDocument = new FakeDocument();
      const currentChild = element(currentDocument, "span");
      const replacementChild = element(replacementDocument, "span");
      const current = element(
        currentDocument,
        "div",
        { "data-suprnova-live-key": `ignored-${modifier}`, [`live:ignore.${modifier}`]: "" },
        [currentChild],
      );
      const replacement = element(
        replacementDocument,
        "div",
        { "data-suprnova-live-key": `ignored-${modifier}`, [`live:ignore.${modifier}`]: "" },
        [replacementChild],
      );
      const prepared = plan({ currentChildren: [current], replacementChildren: [replacement] });
      morph.mockImplementation((_old, _new, options) => {
        expect(
          options.callbacks.beforeNodeMorphed(asElement(current), asElement(replacement)),
        ).toBe(rootSkipped ? false : undefined);
        expect(
          options.callbacks.beforeNodeMorphed(asElement(currentChild), asElement(replacementChild)),
        ).toBe(false);
        expect(options.callbacks.beforeNodeRemoved(asElement(currentChild))).toBe(false);
        expect(options.callbacks.beforeNodeAdded(asElement(replacementChild))).toBe(false);
      });

      new IdiomorphAdapter(() => 0).apply(prepared, {});
      morph.mockReset();
    }
  });

  it("forces declared replacement to dispose and recreate the same logical key", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = element(currentDocument, "div", {
      "data-suprnova-live-key": "replaced",
      "live:replace.subtree": "",
    });
    const replacement = element(replacementDocument, "div", {
      "data-suprnova-live-key": "replaced",
      "live:replace.subtree": "",
    });
    const prepared = plan({ currentChildren: [current], replacementChildren: [replacement] });
    morph.mockImplementation((_old, _new, options) => {
      expect(current.getAttribute("id")).toMatch(/^__suprnova_live_/u);
      expect(replacement.getAttribute("id")).toMatch(/^__suprnova_live_/u);
      expect(current.getAttribute("id")).not.toBe(replacement.getAttribute("id"));
      expect(options.callbacks.beforeNodeRemoved(asElement(current))).toBeUndefined();
      expect(options.callbacks.beforeNodeAdded(asElement(replacement))).toBeUndefined();
    });

    const result = new IdiomorphAdapter(() => 0).apply(prepared, {});
    expect(result.removed).toContain("replaced");
    expect(result.inserted).toContain("replaced");
  });
});
