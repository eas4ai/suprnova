import { describe, expect, it } from "vitest";

import { forcesReplacement, preservesAttribute } from "../src/morph/preserve.js";
import { preflightIslandMorph } from "../src/morph/preflight.js";
import type { MorphPlan } from "../src/morph/types.js";
import {
  asElement,
  element,
  FakeDocument,
  type FakeElement,
  morphFixture,
  text,
} from "./support/morph-dom.js";

// OVL-006: an overlay's open state survives a compatible morph only under a
// stable keyed scope. The library writes every overlay root with a stable
// data-suprnova-live-key and live:preserve.self, so the browser owns the
// root's attributes (the open attribute of details and dialog) while the
// server still morphs its children; an unkeyed root, or one inside a
// replaced region, follows the server.

function disclosure(
  document: FakeDocument,
  attributes: Readonly<Record<string, string>>,
  open: boolean,
  children: readonly FakeElement[] = [],
): FakeElement {
  return element(document, "details", { ...attributes, ...(open ? { open: "" } : {}) }, [
    element(document, "summary", {}, [text(document, "Notes")]),
    ...children,
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

describe("overlay open state across a morph", () => {
  const keyed = { "data-suprnova-live-key": "notes", "live:preserve.self": "" };

  it("keeps a keyed, preserved disclosure open when the server renders it closed", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const { fixture, plan } = planFor(
      [disclosure(currentDocument, keyed, true)],
      [disclosure(replacementDocument, keyed, false)],
    );
    const root = first(fixture.currentRoot);
    expect(preservesAttribute(plan, asElement(root))).toBe(true);
    // The children of the preserved root still come from the server.
    expect(preservesAttribute(plan, asElement(first(root)))).toBe(false);
  });

  it("lets the server close an unkeyed disclosure", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const { fixture, plan } = planFor(
      [disclosure(currentDocument, {}, true)],
      [disclosure(replacementDocument, {}, false)],
    );
    expect(preservesAttribute(plan, asElement(first(fixture.currentRoot)))).toBe(false);
  });

  it("lets the server close a keyed disclosure that is not preserved", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const key = { "data-suprnova-live-key": "notes" };
    const { fixture, plan } = planFor(
      [disclosure(currentDocument, key, true)],
      [disclosure(replacementDocument, key, false)],
    );
    expect(preservesAttribute(plan, asElement(first(fixture.currentRoot)))).toBe(false);
  });

  it("replaces an open disclosure inside a replaced region instead of carrying it over", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const region = (document: FakeDocument, open: boolean) =>
      element(
        document,
        "section",
        { "data-suprnova-live-key": "region", "live:replace.subtree": "" },
        [disclosure(document, keyed, open)],
      );
    const { plan } = planFor([region(currentDocument, true)], [region(replacementDocument, false)]);
    const entry = plan.identity.entries.find((candidate) => candidate.value === "region");
    if (entry === undefined) throw new Error("the region has no identity entry");
    expect(forcesReplacement(plan, entry)).toBe(true);
  });
});
