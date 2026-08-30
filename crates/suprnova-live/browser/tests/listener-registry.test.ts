import { describe, expect, it, vi } from "vitest";

import { DelegatedListenerRegistry } from "../src/runtime/listeners.js";

describe("bounded delegated listener registry", () => {
  it("attaches once across resume cycles and disposes idempotently", () => {
    const target = new EventTarget();
    const listener = vi.fn();
    const registry = new DelegatedListenerRegistry(target);
    const remove = registry.add("click", listener);

    target.dispatchEvent(new Event("click"));
    expect(listener).not.toHaveBeenCalled();
    registry.resume();
    registry.resume();
    target.dispatchEvent(new Event("click"));
    expect(listener).toHaveBeenCalledTimes(1);
    registry.suspend();
    target.dispatchEvent(new Event("click"));
    expect(listener).toHaveBeenCalledTimes(1);
    registry.resume();
    remove();
    remove();
    target.dispatchEvent(new Event("click"));
    expect(listener).toHaveBeenCalledTimes(1);
    registry.dispose();
    registry.dispose();
  });

  it("rejects duplicate phases, unsafe names, and unbounded listener sets", () => {
    const registry = new DelegatedListenerRegistry(new EventTarget());
    registry.add("click", () => undefined);
    expect(() => registry.add("click", () => undefined)).toThrow("listener_registry_rejected");
    expect(() => registry.add("Click", () => undefined)).toThrow("listener_registry_rejected");
    for (let index = 1; index < 32; index += 1) {
      registry.add(`event:${String(index)}`, () => undefined);
    }
    expect(() => registry.add("overflow", () => undefined)).toThrow("listener_registry_rejected");
  });
});
