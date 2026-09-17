import { describe, expect, it, vi } from "vitest";

import type { DirectiveOwnership, OwnedDirective } from "../src/directives/ownership.js";
import type { IslandMetadata } from "../src/islands/metadata.js";
import { IslandRecord } from "../src/islands/record.js";
import { ModelFormRuntime, type ModelDispatch } from "../src/models/forms.js";
import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import type { ServerIntent } from "../src/scheduler/intent.js";

// LIVE-037: an applied render is the baseline a model edit is compared with,
// so an edit that differs from what its control shows is sent even when it
// equals the value the browser last proposed. The controls are stand-ins
// shaped like the inputs the runtime reads; `route` receives the events a
// user's typing fires.

interface NumberControl {
  readonly tagName: "INPUT";
  readonly type: "number";
  value: string;
  readonly disabled: false;
  readonly isConnected: true;
  matches(): boolean;
  hasAttribute(): boolean;
}

interface Harness {
  readonly forms: ModelFormRuntime;
  readonly record: IslandRecord;
  readonly control: NumberControl;
  readonly dispatched: ModelDispatch[];
  readonly timers: VoidFunction[];
  finish(): void;
  type(value: string): void;
  render(value: string, restored?: string): void;
}

function harness(modifiers: readonly string[], holdIntents = false): Harness {
  const control: NumberControl = {
    disabled: false,
    hasAttribute: () => false,
    isConnected: true,
    matches: () => false,
    tagName: "INPUT",
    type: "number",
    value: "1",
  };
  const record = new IslandRecord(
    { setAttribute: vi.fn() } as unknown as Element,
    Object.create(null) as IslandMetadata,
  );
  const owned: OwnedDirective = {
    attributeName: `live:model${modifiers.map((modifier) => `.${modifier}`).join("")}`,
    directive: { modifiers, name: "model", ok: true, value: "seats" },
    element: control as unknown as Element,
    island: record,
  };
  const ownership = { resolveNamed: () => owned } as unknown as DirectiveOwnership;
  const clock: RuntimeClock = { now: () => 0 };
  const timers: VoidFunction[] = [];
  const scheduler: RuntimeScheduler = {
    animationFrame: () => 1,
    cancelAnimationFrame: () => undefined,
    clearTimeout: () => undefined,
    microtask: (callback) => {
      callback();
    },
    timeout: (callback) => {
      timers.push(callback);
      return timers.length;
    },
  };
  const dispatched: ModelDispatch[] = [];
  const finishers: VoidFunction[] = [];
  const forms = new ModelFormRuntime(ownership, clock, scheduler, (dispatch) => {
    dispatched.push(dispatch);
    if (!holdIntents) return null;
    return {
      onFinish: (callback: VoidFunction) => {
        finishers.push(callback);
      },
    } as unknown as ServerIntent;
  });
  forms.connect(record, [owned]);
  return {
    control,
    dispatched,
    finish: () => {
      for (const callback of finishers.splice(0)) callback();
    },
    forms,
    record,
    render: (value, restored = value) => {
      control.value = value;
      const rendered = forms.readRendered(record);
      control.value = restored;
      forms.settleRender(record, rendered);
    },
    timers,
    type: (value) => {
      control.value = value;
      forms.route(
        {
          composedPath: () => [control],
          isTrusted: true,
          type: "input",
        } as unknown as Event,
        "bubble",
      );
    },
  };
}

describe("model edits after an applied render", () => {
  it("LIVE-037: a render that replaces a refused value makes the same value typed again an update", () => {
    const island = harness([]);
    island.type("");
    expect(island.dispatched).toHaveLength(1);
    // The server refused the empty count and a reset renders 1.
    island.render("1");
    island.type("");
    expect(island.dispatched).toHaveLength(2);
    expect(island.dispatched[1]?.batch.proposals).toEqual({ seats: null });
    island.record.dispose();
  });

  it("LIVE-037: the render's value is the accepted value, so a restored local edit stays dirty", () => {
    const island = harness([]);
    island.render("1", "7");
    const state = island.forms.state(island.record);
    expect(state?.snapshot("seats").acceptedServerValue).toBe(1);
    expect(state?.proposal("seats")).toBe(7);
    expect(state?.dirty("seats")).toBe(true);
    island.render("7");
    expect(state?.dirty("seats")).toBe(false);
    island.record.dispose();
  });

  it("LIVE-037: a field whose edit is in flight keeps its newer proposal across a render", () => {
    const island = harness([], true);
    island.type("5");
    const state = island.forms.state(island.record);
    expect(state?.snapshot("seats").inFlightIntent).not.toBeNull();
    island.render("3");
    expect(state?.proposal("seats")).toBe(5);
    island.finish();
    island.render("3");
    expect(state?.proposal("seats")).toBe(3);
    island.record.dispose();
  });

  it("LIVE-037: an edit still waiting on its debounce keeps its proposal across a render", () => {
    const island = harness(["debounce.250ms"]);
    island.type("5");
    expect(island.dispatched).toHaveLength(0);
    island.render("3");
    expect(island.forms.state(island.record)?.proposal("seats")).toBe(5);
    for (const timer of island.timers.splice(0)) timer();
    expect(island.dispatched).toHaveLength(1);
    expect(island.dispatched[0]?.batch.proposals).toEqual({ seats: 5 });
    island.record.dispose();
  });
});
