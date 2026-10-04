import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { EventRouter } from "../src/directives/events.js";
import { DirectiveOwnership } from "../src/directives/ownership.js";
import { parseActionCall, parseDirective } from "../src/directives/parser.js";
import { parseActionParameters, type IslandMetadata } from "../src/islands/metadata.js";
import type { IslandRecord } from "../src/islands/record.js";
import type { RuntimeDiagnosticInput } from "../src/runtime/diagnostics.js";
import type { DelegatedListenerRegistry } from "../src/runtime/listeners.js";
import type { RuntimeScheduler } from "../src/runtime/ports.js";
import type { ServerIntent, ServerOperation } from "../src/scheduler/intent.js";

interface GrammarVectors {
  readonly valid: readonly Readonly<{
    value: string;
    name: string;
    arguments: readonly (string | number | boolean | null)[] | null;
  }>[];
  readonly invalid: readonly string[];
}

const VECTORS = JSON.parse(
  readFileSync(
    new URL("../../tests/fixtures/checker/action-call-grammar.json", import.meta.url),
    "utf8",
  ),
) as GrammarVectors;

function element(
  attributes: Readonly<Record<string, string>>,
  children: Element[] = [],
  island = false,
): Element {
  const collection = Object.assign(children, {
    item(index: number) {
      return children[index] ?? null;
    },
  });
  return {
    nodeType: 1,
    attributes: Object.entries(attributes).map(([name, value]) => ({ name, value })),
    children: collection,
    isConnected: true,
    parentElement: null,
    shadowRoot: null,
    matches: (selector: string) => island && selector === "[data-suprnova-live-island]",
    hasAttribute: (name: string) => name in attributes,
    getAttribute: (name: string) => attributes[name] ?? null,
    setAttribute: () => undefined,
  } as unknown as Element;
}

const SCHEDULER: RuntimeScheduler = {
  animationFrame: () => 1,
  cancelAnimationFrame: () => undefined,
  clearTimeout: () => undefined,
  microtask: (callback) => {
    callback();
  },
  timeout: () => 1,
};

// One island whose only control carries `live:click="{value}"`, wired to
// the real ownership scan and event router, with the request it schedules
// captured instead of sent.
function clickHarness(value: string, actionParameters?: ReadonlyMap<string, readonly string[]>) {
  const button = element({ "live:click": value, type: "button" });
  const root = element({ "data-suprnova-live-island": "" }, [button], true);
  const metadata = {
    component: "fixture.list",
    documentKey: "list",
    instanceId: "MDEyMzQ1Njc4OTo7PD0-Pw",
    lazyComplete: false,
    protocolMinimum: 2,
    revision: 1n,
    runtimeContract: 1,
    slot: "list",
    snapshot: Object.freeze({}),
    snapshotForm: "instance",
    ...(actionParameters === undefined ? {} : { actionParameters }),
  } as IslandMetadata;
  const intents: ServerIntent[] = [];
  const record = {
    element: root,
    metadata,
    active: () => true,
    enqueue: (intent: ServerIntent) => {
      intents.push(intent);
      return true;
    },
    onDispose: () => undefined,
  } as unknown as IslandRecord;
  const listeners = new Map<string, EventListener>();
  const registry = {
    add(type: string, listener: EventListener, options: Readonly<{ capture?: boolean }> = {}) {
      listeners.set(`${type}:${options.capture === true ? "capture" : "bubble"}`, listener);
      return () => undefined;
    },
  } as unknown as DelegatedListenerRegistry;
  const diagnostics: RuntimeDiagnosticInput[] = [];
  const ownership = new DirectiveOwnership();
  const router = new EventRouter(
    registry,
    ownership,
    { randomBytes: (length) => new Uint8Array(length).fill(7) },
    { now: () => 0 },
    SCHEDULER,
    {
      record(diagnostic: RuntimeDiagnosticInput) {
        diagnostics.push(diagnostic);
      },
    },
  );
  router.connect(record, ownership.connect(record));
  const click = () => {
    const event = {
      type: "click",
      isTrusted: true,
      defaultPrevented: false,
      target: button,
      composedPath: () => [button, root],
      preventDefault: () => undefined,
      stopPropagation: () => undefined,
    } as unknown as Event;
    listeners.get("click:capture")?.(event);
    listeners.get("click:bubble")?.(event);
  };
  const invoked = (): ServerOperation | undefined =>
    intents
      .flatMap((intent) => intent.operations)
      .find((operation) => operation.kind === "invoke_action");
  return { click, intents, diagnostics, invoked };
}

describe("action directive arguments", () => {
  it("parses every shared action call vector as the checker does", () => {
    for (const vector of VECTORS.valid) {
      const call = parseActionCall(vector.value);
      expect(call, vector.value).not.toBeNull();
      expect(call?.name, vector.value).toBe(vector.name);
      expect(call?.arguments, vector.value).toEqual(vector.arguments ?? undefined);
    }
    for (const value of VECTORS.invalid) {
      expect(parseActionCall(value), value).toBeNull();
    }
    expect(
      parseActionCall(`save(${Array.from({ length: 128 }, () => "1").join(",")})`),
    ).not.toBeNull();
    expect(parseActionCall(`save(${Array.from({ length: 129 }, () => "1").join(",")})`)).toBeNull();
  });

  it("keeps the bare action name as the directive value and adds the literals", () => {
    expect(parseDirective("live:click", "remove(42)")).toEqual({
      ok: true,
      name: "click",
      value: "remove",
      modifiers: [],
      arguments: [42],
    });
    expect(parseDirective("live:click.prevent", "save")).toEqual({
      ok: true,
      name: "click",
      value: "save",
      modifiers: ["prevent"],
    });
    expect(parseDirective("live:submit", "rename('draft', true)")).toMatchObject({
      ok: true,
      value: "rename",
      arguments: ["draft", true],
    });
    expect(parseDirective("live:click", "remove(42")).toMatchObject({
      ok: false,
      code: "invalid_value",
    });
    expect(parseDirective("live:loading", "remove(42)")).toMatchObject({ ok: false });
  });

  it('sends live:click="remove(42)" as the arguments { id: 42 }', () => {
    const harness = clickHarness("remove(42)", new Map([["remove", ["id"]]]));
    harness.click();
    expect(harness.invoked()).toEqual({
      kind: "invoke_action",
      name: "remove",
      arguments: { id: 42 },
    });
  });

  it("maps literals to the declared parameters in order", () => {
    const harness = clickHarness(
      "rename('draft', true)",
      new Map([["rename", ["title", "publish"]]]),
    );
    harness.click();
    expect(harness.invoked()).toEqual({
      kind: "invoke_action",
      name: "rename",
      arguments: { title: "draft", publish: true },
    });
  });

  it("sends a bare action name with no arguments, as before", () => {
    const harness = clickHarness("save");
    harness.click();
    expect(harness.invoked()).toEqual({ kind: "invoke_action", name: "save", arguments: {} });
  });

  it("refuses a literal no declared parameter receives without sending anything", () => {
    for (const [value, parameters] of [
      ["remove(1, 2)", new Map([["remove", ["id"]]])],
      ["save(1)", undefined],
    ] as const) {
      const harness = clickHarness(value, parameters);
      harness.click();
      expect(harness.intents, value).toEqual([]);
      expect(harness.diagnostics, value).toEqual([
        {
          code: "directive_invalid",
          severity: "error",
          phase: "directive",
          detailCode: "operation_rejected",
        },
      ]);
    }
  });

  it("reads the island root's action parameter list strictly", () => {
    expect(parseActionParameters("remove(id) rename(title,publish)")).toEqual(
      new Map([
        ["remove", ["id"]],
        ["rename", ["title", "publish"]],
      ]),
    );
    for (const invalid of [
      "",
      "remove()",
      "remove(id)  rename(title)",
      " remove(id)",
      "remove(id) remove(other)",
      "remove(id,id)",
      "remove(id",
      "remove id",
      "re move(id)",
      "remove(i d)",
    ]) {
      expect(() => parseActionParameters(invalid), invalid).toThrow("island_invalid");
    }
  });
});
