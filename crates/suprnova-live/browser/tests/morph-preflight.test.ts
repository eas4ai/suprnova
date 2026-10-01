import { describe, expect, it } from "vitest";

import { DEFAULT_MORPH_LIMITS } from "../src/morph/limits.js";
import { MorphPreflightError, preflightIslandMorph } from "../src/morph/preflight.js";
import {
  asElement,
  element,
  FakeDocument,
  morphFixture,
  rootAttributes,
  text,
  withLimits,
} from "./support/morph-dom.js";

function preflight(fixture: ReturnType<typeof morphFixture>, html = "<section></section>") {
  return preflightIslandMorph({
    authority: fixture.authority,
    currentRoot: asElement(fixture.currentRoot),
    html,
    limits: fixture.limits,
    parser: fixture.parser,
  });
}

describe("Live-owned morph preflight", () => {
  it("accepts one inert matching successor and returns an immutable plan", () => {
    const fixture = morphFixture();
    const plan = preflight(fixture);

    expect(plan.currentRoot).toBe(fixture.currentRoot);
    expect(plan.replacementRoot).toBe(fixture.replacementRoot);
    expect(Object.isFrozen(plan)).toBe(true);
  });

  it("accepts a seed root promoting into the successor's instance", () => {
    const fixture = morphFixture({
      currentOverrides: {
        "data-suprnova-live-revision": "0",
        "data-suprnova-live-snapshot-kind": "seed",
      },
    });
    fixture.currentRoot.removeAttribute("data-suprnova-live-instance");

    const plan = preflight(fixture);

    expect(plan.currentRoot).toBe(fixture.currentRoot);
    expect(plan.replacementRoot).toBe(fixture.replacementRoot);
  });

  it("rejects a seed root that already claims an instance or a later revision", () => {
    const claimed = morphFixture({
      currentOverrides: {
        "data-suprnova-live-revision": "0",
        "data-suprnova-live-snapshot-kind": "seed",
      },
    });
    expect(() => preflight(claimed)).toThrow(MorphPreflightError);

    const advanced = morphFixture({
      currentOverrides: {
        "data-suprnova-live-revision": "3",
        "data-suprnova-live-snapshot-kind": "seed",
      },
    });
    advanced.currentRoot.removeAttribute("data-suprnova-live-instance");
    expect(() => preflight(advanced)).toThrow(MorphPreflightError);
  });

  it("rejects empty, multiple-root, parser-error, and parser-failure input", () => {
    const empty = morphFixture();
    expect(() => preflight(empty, "")).toThrow(MorphPreflightError);

    const multiple = morphFixture();
    multiple.replacementDocument.body.append(element(multiple.replacementDocument, "aside"));
    expect(() => preflight(multiple)).toThrow(MorphPreflightError);

    const parserError = morphFixture({
      replacementChildren: [element(new FakeDocument(), "parsererror")],
    });
    expect(() => preflight(parserError)).toThrow(MorphPreflightError);

    const fixture = morphFixture();
    expect(() =>
      preflightIslandMorph({
        ...fixture,
        currentRoot: asElement(fixture.currentRoot),
        html: "<section>",
        parser: {
          parseFromString: () => {
            throw new Error("parser");
          },
        },
      }),
    ).toThrow(MorphPreflightError);
  });

  it.each([
    ["data-suprnova-live-component", "forged.component"],
    ["data-suprnova-live-slot", "forged-slot"],
    ["data-suprnova-live-document-key", "other-document"],
    ["data-suprnova-live-instance", "EBESExQVFhcYGRobHB0eHw"],
    ["data-suprnova-live-revision", "9"],
    ["data-suprnova-live-snapshot", "forged-snapshot"],
  ])("rejects successor authority mismatch in %s", (name, value) => {
    const fixture = morphFixture({ replacementOverrides: { [name]: value } });
    expect(() => preflight(fixture)).toThrow(MorphPreflightError);
  });

  it("rejects executable structures and event attributes before mutation", () => {
    for (const unsafe of [
      (document: FakeDocument) => element(document, "script"),
      (document: FakeDocument) => element(document, "iframe"),
      (document: FakeDocument) => element(document, "button", { onclick: "attack()" }),
      (document: FakeDocument) => element(document, "iframe", { srcdoc: "<script></script>" }),
    ]) {
      const document = new FakeDocument();
      const child = unsafe(document);
      const fixture = morphFixture({ replacementChildren: [child] });
      child.ownerDocument = fixture.replacementDocument;
      expect(() => preflight(fixture)).toThrow(MorphPreflightError);
    }
  });

  it("rejects a replacement tree containing a node from another document", () => {
    const foreign = new FakeDocument();
    const child = element(foreign, "p");
    const fixture = morphFixture({ replacementChildren: [child] });
    child.ownerDocument = foreign;
    expect(() => preflight(fixture)).toThrow(MorphPreflightError);
  });

  it("treats a surviving nested island as opaque and rejects root mutation", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const currentChild = element(
      currentDocument,
      "article",
      rootAttributes("3", "child-snapshot", {
        "data-suprnova-live-component": "catalog.child",
        "data-suprnova-live-document-key": "child",
        "data-suprnova-live-instance": "EBESExQVFhcYGRobHB0eHw",
        "data-suprnova-live-root": "child-slot",
        "data-suprnova-live-slot": "child-slot",
      }),
      [element(currentDocument, "p", {}, [text(currentDocument, "current")])],
    );
    const replacementChild = element(
      replacementDocument,
      "article",
      rootAttributes("4", "forged-child-snapshot", {
        "data-suprnova-live-component": "catalog.child",
        "data-suprnova-live-document-key": "child",
        "data-suprnova-live-instance": "EBESExQVFhcYGRobHB0eHw",
        "data-suprnova-live-root": "child-slot",
        "data-suprnova-live-slot": "child-slot",
      }),
    );
    const fixture = morphFixture({
      currentChildren: [currentChild],
      replacementChildren: [replacementChild],
    });
    currentChild.ownerDocument = fixture.currentDocument;
    replacementChild.ownerDocument = fixture.replacementDocument;
    expect(() => preflight(fixture)).toThrow(MorphPreflightError);
  });

  it.each([
    ["byte", withLimits({ maxHtmlBytes: 1 }), "<section></section>"],
    ["node", withLimits({ maxNodes: 1 }), "<section></section>"],
    ["depth", withLimits({ maxDepth: 1 }), "<section></section>"],
    ["attribute", withLimits({ maxAttributes: 11 }), "<section></section>"],
  ])("enforces the %s limit", (_name, limits, html) => {
    const document = new FakeDocument();
    const child = element(document, "span");
    const fixture = morphFixture({ limits, replacementChildren: [child] });
    child.ownerDocument = fixture.replacementDocument;
    expect(() => preflight(fixture, html)).toThrow(MorphPreflightError);
  });

  it("enforces key syntax, bytes, count, uniqueness, and nested-key agreement", () => {
    const invalidCases = [
      [withLimits({ maxKeyBytes: 4 }), [{ "data-suprnova-live-key": "abcde" }]],
      [withLimits({ maxKeys: 1 }), [{ id: "one" }, { id: "two" }]],
      [
        DEFAULT_MORPH_LIMITS,
        [{ "data-suprnova-live-key": "same" }, { "data-suprnova-live-key": "same" }],
      ],
    ] as const;
    for (const [limits, attributes] of invalidCases) {
      const document = new FakeDocument();
      const children = attributes.map((value) => element(document, "div", value));
      const fixture = morphFixture({ limits, replacementChildren: children });
      for (const child of children) child.ownerDocument = fixture.replacementDocument;
      expect(() => preflight(fixture)).toThrow(MorphPreflightError);
    }

    const nestedDocument = new FakeDocument();
    const nested = element(nestedDocument, "article", {
      ...rootAttributes("0", "child", { "data-suprnova-live-document-key": "child" }),
      "data-suprnova-live-key": "different",
    });
    const nestedFixture = morphFixture({ replacementChildren: [nested] });
    nested.ownerDocument = nestedFixture.replacementDocument;
    expect(() => preflight(nestedFixture)).toThrow(MorphPreflightError);
  });

  it("reports stable-key moves and treats a changed key as new identity", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = [
      element(currentDocument, "li", { "data-suprnova-live-key": "alpha" }),
      element(currentDocument, "li", { "data-suprnova-live-key": "beta" }),
      element(currentDocument, "li", { "data-suprnova-live-key": "old" }),
    ];
    const replacement = [
      element(replacementDocument, "li", { "data-suprnova-live-key": "beta" }),
      element(replacementDocument, "li", { "data-suprnova-live-key": "alpha" }),
      element(replacementDocument, "li", { "data-suprnova-live-key": "new" }),
    ];
    const fixture = morphFixture({ currentChildren: current, replacementChildren: replacement });
    for (const child of current) child.ownerDocument = fixture.currentDocument;
    for (const child of replacement) child.ownerDocument = fixture.replacementDocument;

    const plan = preflight(fixture);
    expect(plan.identity.moved).toEqual(["alpha", "beta"]);
    expect(plan.identity.inserted).toEqual(["new"]);
    expect(plan.identity.removed).toEqual(["old"]);
  });
});
