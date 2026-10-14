import { describe, expect, it } from "vitest";

import { preflightIslandMorph } from "../src/morph/preflight.js";
import type { TeleportTargetPort } from "../src/morph/teleport.js";
import { asElement, element, FakeDocument, morphFixture } from "./support/morph-dom.js";

const key = (value: string): Readonly<Record<string, string>> => ({
  "data-suprnova-live-key": value,
});

function controlled(document: FakeDocument, identity: string, attribute: string, value = "") {
  return element(document, "div", { ...key(identity), [attribute]: value });
}

function preflight(
  currentChildren: readonly ReturnType<typeof element>[],
  replacementChildren: readonly ReturnType<typeof element>[],
  teleports?: TeleportTargetPort,
) {
  const fixture = morphFixture({ currentChildren, replacementChildren });
  return preflightIslandMorph({
    authority: fixture.authority,
    currentRoot: asElement(fixture.currentRoot),
    html: "<section></section>",
    limits: fixture.limits,
    parser: fixture.parser,
    ...(teleports === undefined ? {} : { teleports }),
  });
}

describe("morph preservation controls", () => {
  it("produces distinct bounded control semantics for every supported control", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = [
      controlled(currentDocument, "preserved", "live:preserve.self"),
      controlled(currentDocument, "ignored", "live:ignore.children"),
      controlled(currentDocument, "replaced", "live:replace.subtree"),
      controlled(currentDocument, "persisted", "live:persist", "draft-panel"),
      controlled(currentDocument, "teleported", "live:teleport", "#modal-root"),
    ];
    const replacement = [
      controlled(replacementDocument, "preserved", "live:preserve.self"),
      controlled(replacementDocument, "ignored", "live:ignore.children"),
      controlled(replacementDocument, "replaced", "live:replace.subtree"),
      controlled(replacementDocument, "persisted", "live:persist", "draft-panel"),
      controlled(replacementDocument, "teleported", "live:teleport", "#modal-root"),
    ];
    let target: Element | null = null;
    const plan = preflight(current, replacement, {
      resolve: (selector, ownerRoot) => {
        if (selector !== "#modal-root") return null;
        target ??= element(ownerRoot.ownerDocument as unknown as FakeDocument, "div", {
          id: "modal-root",
        }) as unknown as Element;
        return target;
      },
    });

    expect(plan.controls.bindings.map(({ control }) => control)).toEqual([
      { kind: "preserve", key: "preserved" },
      { attributes: "server", kind: "ignore", key: "ignored" },
      { kind: "replace", key: "replaced" },
      { destination: "draft-panel", kind: "persist", key: "persisted" },
      { key: "teleported", kind: "teleport", target: "#modal-root" },
    ]);
    expect(plan.controls.teleportTargets.get("teleported")).toBe(target);
  });

  it.each([
    [{ "live:preserve.self": "" }],
    [{ ...key("wrong-preserve"), "live:preserve.children": "" }],
    [{ ...key("wrong-ignore"), "live:ignore.self": "" }],
    [{ ...key("wrong-replace"), "live:replace.self": "" }],
    [{ ...key("multiple"), "live:persist": "draft", "live:teleport": "#modal-root" }],
    [{ ...key("unsafe"), "live:teleport": "/route" }],
  ])("rejects missing identity, unsafe modes, combinations, and selector forms", (attributes) => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = element(currentDocument, "div", attributes);
    const replacement = element(replacementDocument, "div", attributes);
    expect(() => preflight([current], [replacement], { resolve: () => null })).toThrow();
  });

  it("rejects control drift, unresolved targets, cross-document targets, and ignored authority", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    expect(() =>
      preflight(
        [controlled(currentDocument, "stable", "live:preserve.self")],
        [controlled(replacementDocument, "stable", "live:replace.subtree")],
      ),
    ).toThrow();

    expect(() =>
      preflight(
        [controlled(currentDocument, "teleported", "live:teleport", "#missing")],
        [controlled(replacementDocument, "teleported", "live:teleport", "#missing")],
        { resolve: () => null },
      ),
    ).toThrow();

    const foreignTarget = element(new FakeDocument(), "div", { id: "modal-root" });
    expect(() =>
      preflight(
        [controlled(currentDocument, "teleported", "live:teleport", "#modal-root")],
        [controlled(replacementDocument, "teleported", "live:teleport", "#modal-root")],
        { resolve: () => foreignTarget as unknown as Element },
      ),
    ).toThrow();

    const currentIgnored = controlled(currentDocument, "ignored", "live:ignore.children");
    currentIgnored.append(element(currentDocument, "button", { "live:click": "save" }));
    const replacementIgnored = controlled(replacementDocument, "ignored", "live:ignore.children");
    replacementIgnored.append(element(replacementDocument, "button", { "live:click": "save" }));
    expect(() => preflight([currentIgnored], [replacementIgnored])).toThrow();
  });

  it("rejects adding or removing a control while the same logical identity survives", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    expect(() =>
      preflight(
        [controlled(currentDocument, "stable", "live:preserve.self")],
        [element(replacementDocument, "div", key("stable"))],
      ),
    ).toThrow();
    expect(() =>
      preflight(
        [element(currentDocument, "div", key("plain"))],
        [controlled(replacementDocument, "plain", "live:preserve.self")],
      ),
    ).toThrow();
  });
});
