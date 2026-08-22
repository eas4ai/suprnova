import { describe, expect, it } from "vitest";

import {
  OptimisticProjectionManager,
  type ProjectionIntent,
  type ProjectionPatch,
} from "../src/extensions/projection.js";
import { RuntimeDiagnostics } from "../src/runtime/diagnostics.js";
import type { RuntimeScheduler } from "../src/runtime/ports.js";

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
  fire(): void {
    for (const callback of [...this.timeouts.values()]) callback();
  }
}

function intent(): { readonly value: ProjectionIntent; finish(): void } {
  let callback: VoidFunction | undefined;
  const value = Object.freeze({
    onFinish(candidate: VoidFunction) {
      callback = candidate;
    },
  });
  return {
    value,
    finish: () => {
      callback?.();
    },
  };
}

function patch(declaration: string, trace: string[], connected = () => true): ProjectionPatch {
  return {
    declaration,
    connected,
    applyPending() {
      trace.push(`apply:${declaration}`);
    },
    rollback() {
      trace.push(`rollback:${declaration}`);
    },
  };
}

function manager(scheduler: RuntimeScheduler): OptimisticProjectionManager {
  return new OptimisticProjectionManager({
    diagnostics: new RuntimeDiagnostics({ mode: "verbose" }),
    scheduler,
    timeoutMs: 50,
  });
}

describe("optimistic projection lifecycle", () => {
  it("keeps accepted state and rolls back rejected state in reverse order", () => {
    const scheduler = new ManualScheduler();
    const projections = manager(scheduler);
    const acceptedTrace: string[] = [];
    const accepted = intent();
    const acceptedHandle = projections.begin(
      accepted.value,
      new Set(["class:pending", "attr:aria-busy"]),
      [patch("class:pending", acceptedTrace), patch("attr:aria-busy", acceptedTrace)],
    );
    expect(acceptedHandle.state()).toBe("pending");
    expect(acceptedHandle.settle("accepted_html")).toBe("settled");
    expect(acceptedTrace).toEqual(["apply:class:pending", "apply:attr:aria-busy"]);

    const rejectedTrace: string[] = [];
    const rejected = intent();
    const rejectedHandle = projections.begin(
      rejected.value,
      new Set(["class:pending", "attr:aria-busy"]),
      [patch("class:pending", rejectedTrace), patch("attr:aria-busy", rejectedTrace)],
    );
    expect(rejectedHandle.settle("rejected")).toBe("settled");
    expect(rejectedTrace).toEqual([
      "apply:class:pending",
      "apply:attr:aria-busy",
      "rollback:attr:aria-busy",
      "rollback:class:pending",
    ]);
  });

  it("rolls back on timeout and intent cancellation", () => {
    const scheduler = new ManualScheduler();
    const projections = manager(scheduler);
    const timeoutTrace: string[] = [];
    const timed = intent();
    const timedHandle = projections.begin(timed.value, new Set(["class:pending"]), [
      patch("class:pending", timeoutTrace),
    ]);
    scheduler.fire();
    expect(timedHandle.state()).toBe("settled");
    expect(timeoutTrace).toEqual(["apply:class:pending", "rollback:class:pending"]);

    const cancelTrace: string[] = [];
    const canceled = intent();
    const canceledHandle = projections.begin(canceled.value, new Set(["class:pending"]), [
      patch("class:pending", cancelTrace),
    ]);
    canceled.finish();
    expect(canceledHandle.state()).toBe("settled");
    expect(cancelTrace).toEqual(["apply:class:pending", "rollback:class:pending"]);
  });

  it("rejects undeclared targets and requests recovery when rollback identity disappeared", () => {
    const scheduler = new ManualScheduler();
    const projections = manager(scheduler);
    const rejected = intent();
    expect(() =>
      projections.begin(rejected.value, new Set(["class:allowed"]), [patch("class:forged", [])]),
    ).toThrow("projection_target_undeclared");

    const removed = intent();
    let connected = true;
    const removedHandle = projections.begin(removed.value, new Set(["class:pending"]), [
      patch("class:pending", [], () => connected),
    ]);
    connected = false;
    expect(removedHandle.settle("interrupted")).toBe("recovery_required");
    expect(removedHandle.state()).toBe("recovery_required");
  });
});
