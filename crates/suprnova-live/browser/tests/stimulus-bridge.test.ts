import { describe, expect, it } from "vitest";

import { createStimulusMorphBridge, type StimulusApplicationPort } from "../src/stimulus/bridge.js";
import { RuntimeDiagnostics } from "../src/runtime/diagnostics.js";
import type { StimulusBootstrapOptions } from "../src/stimulus/port.js";

const ISLAND_ATTRIBUTE = "data-suprnova-live-island";
const CONTROLLER_ATTRIBUTE = "data-controller";
const DOCUMENT_KEY_ATTRIBUTE = "data-suprnova-live-document-key";
const LIVE_KEY_ATTRIBUTE = "data-suprnova-live-key";

class FakeElement {
  readonly nodeType = 1;
  readonly children: FakeElement[] = [];
  parentElement: FakeElement | null = null;

  constructor(readonly attributes: Readonly<Record<string, string>> = {}) {}

  append(...children: FakeElement[]): this {
    for (const child of children) {
      child.parentElement = this;
      this.children.push(child);
    }
    return this;
  }

  getAttribute(name: string): string | null {
    return this.attributes[name] ?? null;
  }

  hasAttribute(name: string): boolean {
    return Object.prototype.hasOwnProperty.call(this.attributes, name);
  }

  matches(selector: string): boolean {
    if (selector === `[${CONTROLLER_ATTRIBUTE}]`) return this.hasAttribute(CONTROLLER_ATTRIBUTE);
    if (selector === `[${ISLAND_ATTRIBUTE}]`) return this.hasAttribute(ISLAND_ATTRIBUTE);
    return false;
  }

  closest(selector: string): FakeElement | null {
    if (this.matches(selector)) return this;
    return this.parentElement?.closest(selector) ?? null;
  }

  querySelectorAll(selector: string): readonly FakeElement[] {
    const matches: FakeElement[] = [];
    const visit = (element: FakeElement) => {
      for (const child of element.children) {
        if (child.matches(selector)) matches.push(child);
        visit(child);
      }
    };
    visit(this);
    return matches;
  }
}

function diagnostics(): RuntimeDiagnostics {
  return new RuntimeDiagnostics({ mode: "verbose" });
}

function element(attributes: Readonly<Record<string, string>> = {}): FakeElement {
  return new FakeElement(attributes);
}

describe("application-supplied Stimulus lifecycle", () => {
  it("loads definitions, starts once, and stops exactly once", () => {
    const trace: string[] = [];
    const application: StimulusApplicationPort = {
      load(...definitions) {
        trace.push(`load:${String(definitions.length)}`);
      },
      start() {
        trace.push("start");
      },
      unload(...identifiers) {
        trace.push(`unload:${identifiers.join(",")}`);
      },
      stop() {
        trace.push("stop");
      },
    };
    const bridge = createStimulusMorphBridge(
      { application, definitions: [{ identifier: "menu" }, { identifier: "dialog" }] },
      diagnostics(),
    );

    expect(trace).toEqual(["load:2", "start"]);
    bridge.dispose();
    bridge.dispose();
    expect(trace).toEqual(["load:2", "start", "unload:menu,dialog", "stop"]);
  });

  it("captures only stable controller identity owned by the current island", () => {
    const runtimeDiagnostics = diagnostics();
    const bridge = createStimulusMorphBridge(
      {
        application: {
          load() {
            return undefined;
          },
          start() {
            return undefined;
          },
          unload() {
            return undefined;
          },
          stop() {
            return undefined;
          },
        },
      },
      runtimeDiagnostics,
    );
    const parent = element({
      [ISLAND_ATTRIBUTE]: "",
      [DOCUMENT_KEY_ATTRIBUTE]: "parent",
    });
    const stable = element({
      [CONTROLLER_ATTRIBUTE]: "menu",
      [LIVE_KEY_ATTRIBUTE]: "stable-menu",
    });
    const unkeyed = element({ [CONTROLLER_ATTRIBUTE]: "tooltip" });
    const childIsland = element({
      [ISLAND_ATTRIBUTE]: "",
      [DOCUMENT_KEY_ATTRIBUTE]: "child",
    });
    const childController = element({
      [CONTROLLER_ATTRIBUTE]: "dialog",
      [LIVE_KEY_ATTRIBUTE]: "child-dialog",
    });
    childIsland.append(childController);
    parent.append(stable, unkeyed, childIsland);

    const continuity = bridge.beforeMorph(parent as unknown as Element);
    expect(continuity.roots.map(({ identity }) => identity)).toEqual(["stable-menu"]);
    bridge.afterMorph(continuity, parent as unknown as Element);
    bridge.afterMorph(continuity, parent as unknown as Element);

    expect(runtimeDiagnostics.entries()).toContainEqual(
      expect.objectContaining({
        code: "lifecycle_notice",
        detailCode: "operation_rejected",
        phase: "lifecycle",
      }),
    );
  });

  it("isolates invalid ports and retired-scope continuity from Live control flow", () => {
    const runtimeDiagnostics = diagnostics();
    const application: StimulusApplicationPort = {
      load() {
        throw new Error("definition detail must not escape");
      },
      start() {
        throw new Error("startup detail must not escape");
      },
      unload() {
        throw new Error("unload detail must not escape");
      },
      stop() {
        throw new Error("shutdown detail must not escape");
      },
    };
    const bridge = createStimulusMorphBridge(
      { application, definitions: [{ identifier: "broken" }] },
      runtimeDiagnostics,
    );
    const scope = element({ [ISLAND_ATTRIBUTE]: "", [DOCUMENT_KEY_ATTRIBUTE]: "scope" });
    const continuity = bridge.beforeMorph(scope as unknown as Element);

    bridge.disposeScope(scope as unknown as Element);
    expect(() => {
      bridge.afterMorph(continuity, scope as unknown as Element);
    }).not.toThrow();
    expect(() => {
      bridge.dispose();
    }).not.toThrow();
    expect(runtimeDiagnostics.entries().length).toBeGreaterThanOrEqual(4);
    expect(
      runtimeDiagnostics
        .entries()
        .every(({ code, phase }) => code === "lifecycle_notice" && phase === "lifecycle"),
    ).toBe(true);
  });

  it("does not retain continuities after disposal and isolates malformed bootstrap input", () => {
    const runtimeDiagnostics = diagnostics();
    expect(() =>
      createStimulusMorphBridge(null as unknown as StimulusBootstrapOptions, runtimeDiagnostics),
    ).not.toThrow();

    const bridge = createStimulusMorphBridge(
      {
        application: {
          load() {
            return undefined;
          },
          start() {
            return undefined;
          },
          stop() {
            return undefined;
          },
          unload() {
            return undefined;
          },
        },
      },
      runtimeDiagnostics,
    );
    const scope = element({ [ISLAND_ATTRIBUTE]: "", [DOCUMENT_KEY_ATTRIBUTE]: "scope" });
    bridge.dispose();
    for (let index = 0; index < 70; index += 1) {
      bridge.beforeMorph(scope as unknown as Element);
    }
    expect(
      runtimeDiagnostics.entries().some(({ detailCode }) => detailCode === "resource_exhausted"),
    ).toBe(false);
  });
});
