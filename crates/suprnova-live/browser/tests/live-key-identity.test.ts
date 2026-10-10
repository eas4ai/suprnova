import { describe, expect, it } from "vitest";

import { ENGINE_KEY_ATTRIBUTE, LIVE_KEY_ATTRIBUTE, stableKeyOf } from "../src/morph/keys.js";
import { preservesAttribute } from "../src/morph/preserve.js";
import { MorphPreflightError, preflightIslandMorph } from "../src/morph/preflight.js";
import type { MorphPlan } from "../src/morph/types.js";
import {
  asElement,
  element,
  FakeDocument,
  type FakeElement,
  morphFixture,
  text,
} from "./support/morph-dom.js";

// LIVE-024: the stable key a template writes is live:key, the attribute the
// checker validates; the runtime reads it wherever it resolves identity,
// controls and preservation scopes. data-suprnova-live-key stays the
// engine's own spelling, and an element carrying both with different
// values has no single identity.

function disclosure(
  document: FakeDocument,
  attributes: Readonly<Record<string, string>>,
  open: boolean,
): FakeElement {
  return element(document, "details", { ...attributes, ...(open ? { open: "" } : {}) }, [
    element(document, "summary", {}, [text(document, "Notes")]),
    element(document, "p", {}, [text(document, "Body")]),
  ]);
}

function first(parent: FakeElement): FakeElement {
  const child = parent.children[0];
  if (child === undefined) throw new Error("the fixture has no child");
  return child;
}

function planFor(current: FakeElement[], replacement: FakeElement[]) {
  const fixture = morphFixture({ currentChildren: current, replacementChildren: replacement });
  const plan: MorphPlan = preflightIslandMorph({
    authority: fixture.authority,
    currentRoot: asElement(fixture.currentRoot),
    html: "<section></section>",
    limits: fixture.limits,
    parser: fixture.parser,
  });
  return { fixture, plan };
}

describe("LIVE-024: live:key is the stable key the runtime reads", () => {
  it("keeps a disclosure keyed with live:key alone open across a compatible morph", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const keyed = { [LIVE_KEY_ATTRIBUTE]: "notes", "live:preserve.self": "" };
    const { fixture, plan } = planFor(
      [disclosure(currentDocument, keyed, true)],
      [disclosure(replacementDocument, keyed, false)],
    );
    const root = first(fixture.currentRoot);
    expect(plan.controls.byKey.has("notes")).toBe(true);
    expect(preservesAttribute(plan, asElement(root))).toBe(true);
    expect(preservesAttribute(plan, asElement(first(root)))).toBe(false);
  });

  it("still reads the engine's spelling on the roots the engine renders", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const keyed = { [ENGINE_KEY_ATTRIBUTE]: "notes", "live:preserve.self": "" };
    const { fixture, plan } = planFor(
      [disclosure(currentDocument, keyed, true)],
      [disclosure(replacementDocument, keyed, false)],
    );
    expect(plan.controls.byKey.has("notes")).toBe(true);
    expect(preservesAttribute(plan, asElement(first(fixture.currentRoot)))).toBe(true);
  });

  it("gives the same identity to both spellings, so a renamed document keeps its scope", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const { fixture, plan } = planFor(
      [
        disclosure(
          currentDocument,
          { [ENGINE_KEY_ATTRIBUTE]: "notes", "live:preserve.self": "" },
          true,
        ),
      ],
      [
        disclosure(
          replacementDocument,
          { [LIVE_KEY_ATTRIBUTE]: "notes", "live:preserve.self": "" },
          false,
        ),
      ],
    );
    expect(preservesAttribute(plan, asElement(first(fixture.currentRoot)))).toBe(true);
  });

  it("refuses an element that carries both spellings with different values", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const conflicting = { [LIVE_KEY_ATTRIBUTE]: "notes", [ENGINE_KEY_ATTRIBUTE]: "other" };
    let refused: unknown = null;
    try {
      planFor(
        [disclosure(currentDocument, conflicting, true)],
        [disclosure(replacementDocument, conflicting, false)],
      );
    } catch (error: unknown) {
      refused = error;
    }
    expect(refused).toBeInstanceOf(MorphPreflightError);
    expect((refused as MorphPreflightError).detail).toBe("morph_identity_key_conflict");
  });

  it("reads one key from an element that carries both spellings agreeing", () => {
    const document = new FakeDocument();
    const both = element(document, "details", {
      [LIVE_KEY_ATTRIBUTE]: "notes",
      [ENGINE_KEY_ATTRIBUTE]: "notes",
    });
    expect(stableKeyOf(asElement(both))).toBe("notes");
    const alone = element(document, "details", { [LIVE_KEY_ATTRIBUTE]: "notes" });
    expect(stableKeyOf(asElement(alone))).toBe("notes");
    expect(stableKeyOf(asElement(element(document, "details", {})))).toBeNull();
  });
});
