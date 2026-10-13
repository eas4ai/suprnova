import { describe, expect, it } from "vitest";

import { preflightIslandMorph } from "../src/morph/preflight.js";
import { TeleportRegistry } from "../src/morph/teleport.js";
import { asElement, element, FakeDocument, morphFixture } from "./support/morph-dom.js";

describe("document-local teleport authority", () => {
  it("captures only initially unique document-local id targets", () => {
    const document = new FakeDocument();
    const owner = element(document, "section", { "data-suprnova-live-island": "" });
    const target = element(document, "div", { id: "modal-root" });
    document.body.append(owner);
    document.body.append(target);
    const registry = new TeleportRegistry(document as unknown as Document);

    expect(registry.resolve("#modal-root", asElement(owner))).toBe(target);
    expect(registry.resolve("/route", asElement(owner))).toBeNull();

    const late = element(document, "div", { id: "late-root" });
    document.body.append(late);
    expect(registry.resolve("#late-root", asElement(owner))).toBeNull();
  });

  it("rejects duplicate targets and targets owned by another island", () => {
    const duplicateDocument = new FakeDocument();
    const owner = element(duplicateDocument, "section", { "data-suprnova-live-island": "" });
    duplicateDocument.body.append(owner);
    duplicateDocument.body.append(element(duplicateDocument, "div", { id: "duplicate" }));
    duplicateDocument.body.append(element(duplicateDocument, "div", { id: "duplicate" }));
    const duplicates = new TeleportRegistry(duplicateDocument as unknown as Document);
    expect(duplicates.resolve("#duplicate", asElement(owner))).toBeNull();

    const crossDocument = new FakeDocument();
    const source = element(crossDocument, "section", { "data-suprnova-live-island": "" });
    const other = element(crossDocument, "aside", { "data-suprnova-live-island": "" });
    const target = element(crossDocument, "div", { id: "other-target" });
    other.append(target);
    crossDocument.body.append(source);
    crossDocument.body.append(other);
    const registry = new TeleportRegistry(crossDocument as unknown as Document);
    expect(registry.resolve("#other-target", asElement(source))).toBeNull();
  });

  it("mounts, prepares, recommits, and rolls back one logical teleport across morphs", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = element(currentDocument, "div", {
      "aria-labelledby": "dialog-title",
      "data-suprnova-live-key": "dialog",
      "live:teleport": "#modal-root",
    });
    const replacement = element(replacementDocument, "div", {
      "aria-labelledby": "dialog-title",
      "data-suprnova-live-key": "dialog",
      "live:teleport": "#modal-root",
    });
    const currentTitle = element(currentDocument, "h2", { id: "dialog-title" });
    const replacementTitle = element(replacementDocument, "h2", { id: "dialog-title" });
    current.append(currentTitle);
    replacement.append(replacementTitle);
    const fixture = morphFixture({
      currentChildren: [current],
      replacementChildren: [replacement],
    });
    const target = element(fixture.currentDocument, "div", { id: "modal-root" });
    fixture.currentDocument.body.append(target);
    const registry = new TeleportRegistry(fixture.currentDocument as unknown as Document);

    registry.mount(asElement(fixture.currentRoot));
    expect(target.children).toContain(current);
    expect(registry.active(asElement(fixture.currentRoot))).toHaveLength(1);
    expect(registry.consumeControlledMove(current as unknown as Node)).toBe(true);
    expect(registry.consumeControlledMove(current as unknown as Node)).toBe(true);
    expect(registry.consumeControlledMove(current as unknown as Node)).toBe(false);

    const plan = preflightIslandMorph({
      authority: fixture.authority,
      currentRoot: asElement(fixture.currentRoot),
      html: "<section></section>",
      limits: fixture.limits,
      parser: fixture.parser,
      teleports: registry,
    });
    expect(plan.identity.entries.find(({ value }) => value === "dialog")?.current).toBe(current);
    expect(plan.identity.entries.find(({ value }) => value === "dialog-title")?.current).toBe(
      currentTitle,
    );

    const transition = registry.begin(plan);
    expect(fixture.currentRoot.children).toContain(current);
    registry.commit(transition, asElement(fixture.currentRoot));
    expect(target.children).toContain(current);
    expect(current.getAttribute("aria-labelledby")).toBe("dialog-title");

    const repeated = registry.begin(plan);
    registry.rollback(repeated);
    expect(target.children).toContain(current);
    expect(registry.active(asElement(fixture.currentRoot))).toHaveLength(1);
  });

  it("retires an active teleport when the successor removes it", () => {
    const currentDocument = new FakeDocument();
    const current = element(currentDocument, "div", {
      "data-suprnova-live-key": "dialog",
      "live:teleport": "#modal-root",
    });
    const fixture = morphFixture({ currentChildren: [current] });
    const target = element(fixture.currentDocument, "div", { id: "modal-root" });
    fixture.currentDocument.body.append(target);
    const registry = new TeleportRegistry(fixture.currentDocument as unknown as Document);
    registry.mount(asElement(fixture.currentRoot));
    const plan = preflightIslandMorph({
      authority: fixture.authority,
      currentRoot: asElement(fixture.currentRoot),
      html: "<section></section>",
      limits: fixture.limits,
      parser: fixture.parser,
      teleports: registry,
    });

    const transition = registry.begin(plan);
    current.remove();
    registry.commit(transition, asElement(fixture.currentRoot));
    expect(registry.active(asElement(fixture.currentRoot))).toEqual([]);
    expect(target.children).not.toContain(current);
  });

  it("supports an authorized target inside the same island across preflight", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const current = element(currentDocument, "div", {
      "data-suprnova-live-key": "dialog",
      "live:teleport": "#local-modal-root",
    });
    const replacement = element(replacementDocument, "div", {
      "data-suprnova-live-key": "dialog",
      "live:teleport": "#local-modal-root",
    });
    const fixture = morphFixture({
      currentChildren: [current],
      replacementChildren: [replacement],
    });
    const target = element(fixture.currentDocument, "div", { id: "local-modal-root" });
    fixture.currentRoot.append(target);
    const registry = new TeleportRegistry(fixture.currentDocument as unknown as Document);

    registry.mount(asElement(fixture.currentRoot));
    expect(() =>
      preflightIslandMorph({
        authority: fixture.authority,
        currentRoot: asElement(fixture.currentRoot),
        html: "<section></section>",
        limits: fixture.limits,
        parser: fixture.parser,
        teleports: registry,
      }),
    ).not.toThrow();
  });

  it("rejects mutated active identity and rolls back a partially committed transition", () => {
    const currentDocument = new FakeDocument();
    const replacementDocument = new FakeDocument();
    const first = element(currentDocument, "div", {
      "data-suprnova-live-key": "first-dialog",
      "live:teleport": "#first-target",
    });
    const second = element(currentDocument, "div", {
      "data-suprnova-live-key": "second-dialog",
      "live:teleport": "#second-target",
    });
    const firstReplacement = element(replacementDocument, "div", {
      "data-suprnova-live-key": "first-dialog",
      "live:teleport": "#first-target",
    });
    const secondReplacement = element(replacementDocument, "div", {
      "data-suprnova-live-key": "second-dialog",
      "live:teleport": "#second-target",
    });
    const fixture = morphFixture({
      currentChildren: [first, second],
      replacementChildren: [firstReplacement, secondReplacement],
    });
    const firstTarget = element(fixture.currentDocument, "div", { id: "first-target" });
    const secondTarget = element(fixture.currentDocument, "div", { id: "second-target" });
    fixture.currentDocument.body.append(firstTarget);
    fixture.currentDocument.body.append(secondTarget);
    const registry = new TeleportRegistry(fixture.currentDocument as unknown as Document);
    registry.mount(asElement(fixture.currentRoot));
    first.setAttribute("id", "unsafe active identity");
    expect(() =>
      preflightIslandMorph({
        authority: fixture.authority,
        currentRoot: asElement(fixture.currentRoot),
        html: "<section></section>",
        limits: fixture.limits,
        parser: fixture.parser,
        teleports: registry,
      }),
    ).toThrow();
    first.removeAttribute("id");
    const plan = preflightIslandMorph({
      authority: fixture.authority,
      currentRoot: asElement(fixture.currentRoot),
      html: "<section></section>",
      limits: fixture.limits,
      parser: fixture.parser,
      teleports: registry,
    });
    const transition = registry.begin(plan);
    (plan.controls.teleportTargets as Map<string, Element>).delete("second-dialog");
    expect(() => {
      registry.commit(transition, asElement(fixture.currentRoot));
    }).toThrow();
    registry.rollback(transition);
    expect(firstTarget.children).toContain(first);
    expect(secondTarget.children).toContain(second);
    expect(fixture.currentRoot.children).not.toContain(first);
    expect(fixture.currentRoot.children).not.toContain(second);
  });
});
