import { describe, expect, test, vi } from "vitest";

import {
  DocumentLifecycle,
  type DocumentLifecycleCompatibility,
} from "../src/lifecycle/document.js";
import { bindResourceLedger, ResourceLedgerImpl } from "../src/lifecycle/resources.js";
import { lifecycleTestProbe } from "../src/lifecycle/testing.js";

class RecordingTarget extends EventTarget {
  readonly additions: string[] = [];
  readonly removals: string[] = [];

  override addEventListener(
    type: string,
    callback: EventListenerOrEventListenerObject | null,
  ): void {
    this.additions.push(type);
    super.addEventListener(type, callback);
  }

  override removeEventListener(
    type: string,
    callback: EventListenerOrEventListenerObject | null,
  ): void {
    this.removals.push(type);
    super.removeEventListener(type, callback);
  }
}

function transition(type: "pagehide" | "pageshow", persisted: boolean): Event {
  const event = new Event(type);
  Object.defineProperty(event, "persisted", { configurable: false, value: persisted });
  return event;
}

function harness(compatible = true) {
  const window = new RecordingTarget();
  const document = new RecordingTarget();
  const ledger = new ResourceLedgerImpl();
  const calls = { dispose: 0, resume: 0, suspend: 0 };
  const validate = vi.fn(() => compatible);
  const compatibility: DocumentLifecycleCompatibility = { validate };
  ledger.track("controller", {
    dispose: () => {
      calls.dispose += 1;
    },
    resume: () => {
      calls.resume += 1;
    },
    suspend: () => {
      calls.suspend += 1;
    },
  });
  const lifecycle = new DocumentLifecycle({
    compatibility,
    document,
    ledger,
    supportsFreezeResume: true,
    window,
  });
  return { calls, compatibility, document, ledger, lifecycle, validate, window };
}

describe("resource ledger", () => {
  test("owns bounded resources and suspends, resumes, and disposes each exactly once per edge", () => {
    const ledger = new ResourceLedgerImpl({ maxResources: 4 });
    const hooks: string[] = [];
    const listener = ledger.add("listener", () => hooks.push("listener:dispose"));
    ledger.track("transport", {
      dispose: () => hooks.push("transport:dispose"),
      resume: () => hooks.push("transport:resume"),
      suspend: () => hooks.push("transport:suspend"),
    });

    ledger.resume();
    ledger.resume();
    expect(ledger.counts()).toMatchObject({ listener: 1, transport: 1 });
    ledger.suspend();
    ledger.suspend();
    listener.dispose();
    listener.dispose();
    ledger.resume();
    ledger.dispose();
    ledger.dispose();

    expect(hooks).toEqual([
      "transport:resume",
      "transport:suspend",
      "listener:dispose",
      "transport:resume",
      "transport:dispose",
    ]);
    expect(Object.values(ledger.counts()).every((count) => count === 0)).toBe(true);
    expect(() => ledger.add("timer", () => undefined)).toThrow("resource_ledger_disposed");
  });

  test("the non-production probe exposes only closed counts and weak reachability", () => {
    const owner = {};
    const ledger = new ResourceLedgerImpl();
    bindResourceLedger(owner, ledger);
    ledger.add("observer", () => undefined);

    const probe = lifecycleTestProbe(owner);
    expect(probe.counts.observer).toBe(1);
    expect(probe.weak?.deref()).toBe(owner);
  });
});

describe("document lifecycle", () => {
  test("persisted pagehide and freeze suspend once while pageshow restores through one new epoch", () => {
    const { calls, document, lifecycle, validate, window } = harness();
    lifecycle.start();
    const initialEpoch = lifecycle.epoch();

    window.dispatchEvent(transition("pagehide", true));
    document.dispatchEvent(new Event("freeze"));
    expect(lifecycle.state()).toBe("suspended");
    expect(calls.suspend).toBe(1);

    window.dispatchEvent(transition("pageshow", true));
    window.dispatchEvent(transition("pageshow", true));
    document.dispatchEvent(new Event("resume"));

    expect(lifecycle.state()).toBe("active");
    expect(lifecycle.epoch()).toBe(initialEpoch + 1);
    expect(calls.resume).toBe(2);
    expect(validate).toHaveBeenCalledTimes(1);
  });

  test("non-persisted pagehide is true replacement and removes every lifecycle listener", () => {
    const { calls, document, ledger, lifecycle, window } = harness();
    lifecycle.start();
    window.dispatchEvent(transition("pagehide", false));

    expect(lifecycle.state()).toBe("disposed");
    expect(calls.dispose).toBe(1);
    expect(window.additions).toEqual(["pagehide", "pageshow"]);
    expect(document.additions).toEqual(["freeze", "resume"]);
    expect([...window.additions, ...document.additions]).not.toContain("unload");
    expect(window.removals).toEqual(["pageshow", "pagehide"]);
    expect(document.removals).toEqual(["resume", "freeze"]);
    expect(Object.values(ledger.counts()).every((count) => count === 0)).toBe(true);
  });

  test("an incompatible restore disposes behavior while leaving the document to native refresh", () => {
    const { calls, lifecycle, validate, window } = harness(false);
    lifecycle.start();
    window.dispatchEvent(transition("pagehide", true));
    window.dispatchEvent(transition("pageshow", true));

    expect(validate).toHaveBeenCalledOnce();
    expect(lifecycle.state()).toBe("disposed");
    expect(calls.resume).toBe(1);
    expect(calls.dispose).toBe(1);
  });

  test("late callbacks are rejected across suspension, restoration, and disposal", () => {
    const { lifecycle, window } = harness();
    lifecycle.start();
    const callback = vi.fn();
    const oldEpoch = lifecycle.guard(callback);
    oldEpoch("active");

    window.dispatchEvent(transition("pagehide", true));
    oldEpoch("suspended");
    window.dispatchEvent(transition("pageshow", true));
    oldEpoch("restored");
    const restoredEpoch = lifecycle.guard(callback);
    restoredEpoch("current");
    lifecycle.dispose();
    restoredEpoch("disposed");

    expect(callback).toHaveBeenCalledTimes(2);
    expect(callback).toHaveBeenNthCalledWith(1, "active");
    expect(callback).toHaveBeenNthCalledWith(2, "current");
  });

  test("repeated start, suspend, resume, and dispose transitions remain idempotent", () => {
    const { calls, document, lifecycle } = harness();
    lifecycle.start();
    lifecycle.start();
    lifecycle.suspend();
    lifecycle.suspend();
    lifecycle.restore();
    lifecycle.restore();
    lifecycle.dispose();
    lifecycle.dispose();
    document.dispatchEvent(new Event("resume"));

    expect(calls).toEqual({ dispose: 1, resume: 2, suspend: 1 });
    expect(lifecycle.state()).toBe("disposed");
  });
});
