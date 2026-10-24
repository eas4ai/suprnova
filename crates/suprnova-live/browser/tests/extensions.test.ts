import { describe, expect, it } from "vitest";

import type { JsonValue } from "../src/canonical.js";
import { RuntimeCallRegistry, type RuntimeCallContextInput } from "../src/extensions/calls.js";
import {
  EffectRegistry,
  type EffectContextInput,
  type EffectInvocation,
  type EffectRegistration,
} from "../src/extensions/effects.js";
import type { PayloadSchema } from "../src/extensions/schema.js";
import { RuntimeDiagnostics } from "../src/runtime/diagnostics.js";
import type { RuntimeScheduler } from "../src/runtime/ports.js";

const MESSAGE_SCHEMA: PayloadSchema = {
  type: "object",
  properties: { message: { type: "string", maxBytes: 32 } },
  required: ["message"],
  additionalProperties: false,
};

const VALUE_SCHEMA: PayloadSchema = {
  type: "object",
  properties: { value: { type: "integer" } },
  required: ["value"],
  additionalProperties: false,
};

class ManualScheduler implements RuntimeScheduler {
  readonly timeouts = new Map<number, VoidFunction>();
  #next = 1;

  microtask(callback: VoidFunction): void {
    callback();
  }

  animationFrame(callback: FrameRequestCallback): number {
    callback(0);
    return 1;
  }

  cancelAnimationFrame(): void {
    return undefined;
  }

  timeout(callback: VoidFunction): number {
    const handle = this.#next;
    this.#next += 1;
    this.timeouts.set(handle, callback);
    return handle;
  }

  clearTimeout(handle: number): void {
    this.timeouts.delete(handle);
  }

  fireTimeouts(): void {
    for (const callback of [...this.timeouts.values()]) callback();
  }
}

function diagnostics(): RuntimeDiagnostics {
  return new RuntimeDiagnostics({ mode: "verbose" });
}

function effectContext(overrides: Partial<EffectContextInput> = {}): EffectContextInput {
  return {
    active: () => true,
    island: { component: "catalog.search", documentKey: "primary", slot: "results" },
    invokeCall: (_name, input) => Promise.resolve(input),
    phase: "after_commit",
    ...overrides,
  };
}

function callContext(overrides: Partial<RuntimeCallContextInput> = {}): RuntimeCallContextInput {
  return {
    active: () => true,
    island: { component: "catalog.search", documentKey: "primary", slot: "results" },
    local: (_name, input) => Promise.resolve(input),
    server: (_name, input) => Promise.resolve(input),
    ...overrides,
  };
}

describe("closed effect registrations", () => {
  it("registers exact name/version pairs and disposes idempotently", async () => {
    const scheduler = new ManualScheduler();
    const registry = new EffectRegistry({ diagnostics: diagnostics(), scheduler });
    const registration: EffectRegistration = {
      name: "announce",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => undefined,
    };
    const dispose = registry.register(registration);
    expect(() => registry.register(registration)).toThrow("extension_duplicate");
    expect(
      (
        await registry.runAll(effectContext(), [{ name: "announce", payload: { message: "ok" } }])
      )[0],
    ).toMatchObject({ status: "completed", version: 1 });
    dispose();
    dispose();
    expect(
      (
        await registry.runAll(effectContext(), [{ name: "announce", payload: { message: "ok" } }])
      )[0],
    ).toMatchObject({ status: "missing" });
  });

  it("compiles reusable data-only schema nodes without mistaking sharing for a cycle", () => {
    const stringNode = { type: "string", maxBytes: 16 } as const;
    const sharedSchema: PayloadSchema = {
      type: "object",
      properties: { first: stringNode, second: stringNode },
      required: ["first", "second"],
      additionalProperties: false,
    };
    const registry = new EffectRegistry({
      diagnostics: diagnostics(),
      scheduler: new ManualScheduler(),
    });
    expect(() =>
      registry.register({
        name: "shared-schema",
        version: 1,
        schema: sharedSchema,
        phase: "after_commit",
        run: () => undefined,
      }),
    ).not.toThrow();
  });

  it("rejects code/module registrations and validates payload before handlers", async () => {
    const scheduler = new ManualScheduler();
    const registry = new EffectRegistry({ diagnostics: diagnostics(), scheduler });
    const unsafe = {
      name: "unsafe",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => undefined,
      code: "alert(1)",
      moduleUrl: "https://evil.example/effect.js",
    } as unknown as EffectRegistration;
    expect(() => registry.register(unsafe)).toThrow("extension_registration_shape");

    let runs = 0;
    registry.register({
      name: "announce",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => {
        runs += 1;
      },
    });
    const outcomes = await registry.runAll(effectContext(), [
      { name: "announce", payload: { message: 4 } },
      { name: "announce", payload: { message: "x".repeat(33) } },
      {
        name: "announce",
        payload: { message: "forged" },
        moduleUrl: "https://evil.example/effect.js",
      } as unknown as EffectInvocation,
    ]);
    expect(outcomes.map(({ status }) => status)).toEqual(["invalid", "invalid", "invalid"]);
    expect(runs).toBe(0);
  });

  it("scopes failures, continues later effects, and rejects wrong phase/island", async () => {
    const scheduler = new ManualScheduler();
    const registry = new EffectRegistry({ diagnostics: diagnostics(), scheduler });
    const order: string[] = [];
    registry.register({
      name: "throws",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => {
        order.push("throws");
        throw new Error("must-not-leak");
      },
    });
    registry.register({
      name: "continues",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => {
        order.push("continues");
      },
    });
    const outcomes = await registry.runAll(effectContext(), [
      { name: "throws", payload: { message: "first" } },
      { name: "continues", payload: { message: "second" } },
    ]);
    expect(outcomes.map(({ status }) => status)).toEqual(["failed", "completed"]);
    expect(order).toEqual(["throws", "continues"]);
    expect(
      (
        await registry.runAll(effectContext({ phase: "before_commit" }), [
          { name: "continues", payload: { message: "wrong phase" } },
        ])
      )[0],
    ).toMatchObject({ status: "invalid_context" });
    expect(
      (
        await registry.runAll(effectContext({ active: () => false }), [
          { name: "continues", payload: { message: "retired" } },
        ])
      )[0],
    ).toMatchObject({ status: "invalid_context" });
  });

  it("bounds async completion and ignores late work after timeout or disposal", async () => {
    const scheduler = new ManualScheduler();
    const registry = new EffectRegistry({ deadlineMs: 25, diagnostics: diagnostics(), scheduler });
    let complete: (() => void) | undefined;
    let lateContext: Parameters<EffectRegistration["run"]>[0] | undefined;
    let routedCalls = 0;
    const dispose = registry.register({
      name: "slow",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: async (context) =>
        new Promise<void>((resolve) => {
          lateContext = context;
          complete = resolve;
        }),
    });
    const timed = registry.runAll(
      effectContext({
        invokeCall: (_name, input) => {
          routedCalls += 1;
          return Promise.resolve(input);
        },
      }),
      [{ name: "slow", payload: { message: "wait" } }],
    );
    await Promise.resolve();
    scheduler.fireTimeouts();
    expect((await timed)[0]).toMatchObject({ status: "timeout" });
    await expect(lateContext?.call("late", null)).rejects.toThrow("extension_canceled");
    expect(routedCalls).toBe(0);
    complete?.();

    const canceled = registry.runAll(effectContext(), [
      { name: "slow", payload: { message: "dispose" } },
    ]);
    await Promise.resolve();
    dispose();
    expect((await canceled)[0]).toMatchObject({ status: "canceled" });

    let starts = 0;
    const immediateRegistry = new EffectRegistry({
      deadlineMs: 25,
      diagnostics: diagnostics(),
      scheduler,
    });
    const immediateDispose = immediateRegistry.register({
      name: "not-started",
      version: 1,
      schema: MESSAGE_SCHEMA,
      phase: "after_commit",
      run: () => {
        starts += 1;
      },
    });
    const immediate = immediateRegistry.runAll(effectContext(), [
      { name: "not-started", payload: { message: "cancel first" } },
    ]);
    immediateDispose();
    expect((await immediate)[0]).toMatchObject({ status: "canceled" });
    expect(starts).toBe(0);
  });
});

describe("closed public runtime calls", () => {
  it("validates input/output and exposes only owner-bound local/server ports", async () => {
    const scheduler = new ManualScheduler();
    const routes: string[] = [];
    const registry = new RuntimeCallRegistry({ diagnostics: diagnostics(), scheduler });
    registry.register({
      name: "increment",
      input: VALUE_SCHEMA,
      output: VALUE_SCHEMA,
      async run(context, input) {
        const object = input as Readonly<Record<string, JsonValue>>;
        await context.server("increment", input);
        await context.local("count", object["value"] ?? null);
        return { value: Number(object["value"]) + 1 };
      },
    });
    const context = callContext({
      local: (name, input) => {
        routes.push(`local:${name}`);
        return Promise.resolve(input);
      },
      server: (name, input) => {
        routes.push(`server:${name}`);
        return Promise.resolve(input);
      },
    });
    await expect(registry.invoke(context, "increment", { value: 2 })).resolves.toEqual({
      value: 3,
    });
    expect(routes).toEqual(["server:increment", "local:count"]);
    await expect(registry.invoke(context, "missing", { value: 2 })).rejects.toThrow(
      "extension_missing",
    );
    await expect(registry.invoke(context, "increment", { value: "2" })).rejects.toThrow(
      "extension_payload_invalid",
    );
    await expect(
      registry.invoke(callContext({ active: () => false }), "increment", { value: 2 }),
    ).rejects.toThrow("extension_context_invalid");
  });

  it("rejects duplicate calls and invalid handler output", async () => {
    const registry = new RuntimeCallRegistry({
      diagnostics: diagnostics(),
      scheduler: new ManualScheduler(),
    });
    const dispose = registry.register({
      name: "broken",
      input: VALUE_SCHEMA,
      output: VALUE_SCHEMA,
      run: () => ({ value: "wrong" }),
    });
    expect(() =>
      registry.register({
        name: "broken",
        input: VALUE_SCHEMA,
        output: VALUE_SCHEMA,
        run: () => ({ value: 1 }),
      }),
    ).toThrow("extension_duplicate");
    await expect(registry.invoke(callContext(), "broken", { value: 1 })).rejects.toThrow(
      "extension_payload_invalid",
    );
    dispose();
    dispose();
  });
});
