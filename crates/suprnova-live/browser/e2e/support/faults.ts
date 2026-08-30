import type { Page } from "@playwright/test";

export interface ResourceSnapshot {
  readonly listeners: number;
  readonly mutationObservers: number;
  readonly timers: number;
}

export async function dispatchPersistedLifecycle(
  page: Page,
  type: "pagehide" | "pageshow",
): Promise<void> {
  await page.evaluate((eventType) => {
    const event = new Event(eventType);
    Object.defineProperty(event, "persisted", { value: true });
    window.dispatchEvent(event);
  }, type);
}

export async function installResourceInstrumentation(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const state = {
      listeners: new Set<string>(),
      mutationObservers: 0,
      timers: new Set<number>(),
    };
    const listenerIds = new WeakMap<object, number>();
    const targetIds = new WeakMap<object, number>();
    let listenerSequence = 0;
    let targetSequence = 0;
    const listenerId = (listener: EventListenerOrEventListenerObject | null): number => {
      if (listener === null) return 0;
      const owner = listener as object;
      const found = listenerIds.get(owner);
      if (found !== undefined) return found;
      listenerSequence += 1;
      listenerIds.set(owner, listenerSequence);
      return listenerSequence;
    };
    const targetId = (target: EventTarget): number => {
      const owner = target as object;
      const found = targetIds.get(owner);
      if (found !== undefined) return found;
      targetSequence += 1;
      targetIds.set(owner, targetSequence);
      return targetSequence;
    };
    const capture = (options?: boolean | AddEventListenerOptions): boolean =>
      typeof options === "boolean" ? options : (options?.capture ?? false);
    const add: unknown = Reflect.get(EventTarget.prototype, "addEventListener");
    const remove: unknown = Reflect.get(EventTarget.prototype, "removeEventListener");
    if (typeof add !== "function" || typeof remove !== "function") {
      throw new Error("listener_instrumentation_unavailable");
    }
    EventTarget.prototype.addEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      state.listeners.add(
        `${String(targetId(this))}:${type}:${String(listenerId(listener))}:${String(capture(options))}`,
      );
      Reflect.apply(add, this, [type, listener, options]);
    };
    EventTarget.prototype.removeEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      state.listeners.delete(
        `${String(targetId(this))}:${type}:${String(listenerId(listener))}:${String(capture(options))}`,
      );
      Reflect.apply(remove, this, [type, listener, options]);
    };

    const NativeMutationObserver = window.MutationObserver;
    const activeObservers = new WeakSet<MutationObserver>();
    window.MutationObserver = class InstrumentedMutationObserver extends NativeMutationObserver {
      override observe(target: Node, options?: MutationObserverInit): void {
        if (!activeObservers.has(this)) {
          activeObservers.add(this);
          state.mutationObservers += 1;
        }
        super.observe(target, options);
      }

      override disconnect(): void {
        if (activeObservers.delete(this)) {
          state.mutationObservers -= 1;
        }
        super.disconnect();
      }
    };

    // suprnova-correctness-delay-allow: fake-clock -- resource instrumentation resolves the native timer without waiting for correctness
    const timeout: unknown = Reflect.get(window, "setTimeout");
    const clear: unknown = Reflect.get(window, "clearTimeout");
    if (typeof timeout !== "function" || typeof clear !== "function") {
      throw new Error("timer_instrumentation_unavailable");
    }
    const instrumentedTimeout = (
      handler: TimerHandler,
      delay?: number,
      ...arguments_: unknown[]
    ): number => {
      let handle = 0;
      const wrapped = (): void => {
        state.timers.delete(handle);
        if (typeof handler === "function") Reflect.apply(handler, window, arguments_);
      };
      handle = Number(Reflect.apply(timeout, window, [wrapped, delay]));
      state.timers.add(handle);
      return handle;
    };
    Reflect.set(window, "setTimeout", instrumentedTimeout);
    window.clearTimeout = ((handle?: number) => {
      if (typeof handle === "number") state.timers.delete(handle);
      Reflect.apply(clear, window, [handle]);
    }) as typeof window.clearTimeout;

    Reflect.set(window, "__suprnovaResourceSnapshot", () => ({
      listeners: state.listeners.size,
      mutationObservers: state.mutationObservers,
      timers: state.timers.size,
    }));
  });
}

export async function resourceSnapshot(page: Page): Promise<ResourceSnapshot> {
  return page.evaluate(() => {
    const snapshot: unknown = Reflect.get(window, "__suprnovaResourceSnapshot");
    if (typeof snapshot !== "function") throw new Error("resource_snapshot_unavailable");
    const value: unknown = Reflect.apply(snapshot, window, []);
    if (typeof value !== "object" || value === null) throw new Error("resource_snapshot_invalid");
    const listeners: unknown = Reflect.get(value, "listeners");
    const mutationObservers: unknown = Reflect.get(value, "mutationObservers");
    const timers: unknown = Reflect.get(value, "timers");
    if (
      typeof listeners !== "number" ||
      typeof mutationObservers !== "number" ||
      typeof timers !== "number"
    ) {
      throw new Error("resource_snapshot_invalid");
    }
    return { listeners, mutationObservers, timers };
  });
}
