import { describe, expect, it } from "vitest";

import { ChildParameterDeliveryState, queueChildDeliveries } from "../src/application/children.js";
import { dispatchValidatedEvents, runValidatedEffects } from "../src/application/emissions.js";
import { applyUrlReflection, reflectedUrl, UrlReflectionError } from "../src/application/url.js";
import type { IntentSource } from "../src/scheduler/intent.js";
import { ServerIntent } from "../src/scheduler/intent.js";

function paramsChangedIntent(name: string, authority = name): ServerIntent {
  return new ServerIntent(
    Object.freeze({ eventType: name }) as unknown as IntentSource,
    Object.freeze([{ kind: "params_changed" }]),
    null,
    Object.freeze({}),
    Object.freeze({}),
    Object.freeze({
      envelope: Object.freeze({ signature: authority }),
      parent_snapshot: Object.freeze({ body: Object.freeze({ revision: authority }) }),
    }),
  );
}

describe("post-commit application helpers", () => {
  it("schedules A again after A to B is accepted", () => {
    const state = new ChildParameterDeliveryState();
    const initialA = paramsChangedIntent("initial-a");
    expect(state.track("A", initialA)).toBe(true);
    initialA.finish("accepted");
    const changedB = paramsChangedIntent("changed-b");
    expect(state.track("B", changedB)).toBe(true);
    changedB.finish("accepted");

    expect(state.track("A", paramsChangedIntent("return-a"))).toBe(true);
  });

  it("allows a failed parameter delivery to retry the same hash", () => {
    const state = new ChildParameterDeliveryState();
    const failedB = paramsChangedIntent("failed-b");
    expect(state.track("B", failedB)).toBe(true);
    failedB.finish("rejected");

    expect(state.track("B", paramsChangedIntent("retry-b"))).toBe(true);
  });

  it("coalesces a duplicate of the current or pending parameter hash", () => {
    const state = new ChildParameterDeliveryState();
    const pendingB = paramsChangedIntent("pending-b");
    expect(state.track("B", pendingB)).toBe(true);
    expect(state.track("B", paramsChangedIntent("duplicate-pending-b", "pending-b"))).toBe(false);
    pendingB.finish("accepted");

    expect(state.track("B", paramsChangedIntent("duplicate-current-b"))).toBe(false);
  });

  it("preserves newer authority for the same pending hash when the older intent fails", () => {
    const state = new ChildParameterDeliveryState();
    const older = paramsChangedIntent("parent-revision-7");
    const newer = paramsChangedIntent("parent-revision-8");
    expect(state.track("B", older)).toBe(true);
    expect(state.track("B", newer)).toBe(true);
    expect(state.track("B", paramsChangedIntent("duplicate-newer", "parent-revision-8"))).toBe(
      false,
    );

    older.finish("rejected");
    expect(state.track("B", paramsChangedIntent("still-pending-newer", "parent-revision-8"))).toBe(
      false,
    );
    newer.finish("accepted");

    expect(state.track("B", paramsChangedIntent("applied-new-authority"))).toBe(false);
  });

  it("queues signed child deliveries independently without rolling back the parent", () => {
    const parentSnapshot = Object.freeze({
      body: Object.freeze({ revision: "4" }),
      signature: "P".repeat(43),
    });
    const queued: Readonly<{ hash: string; parentSnapshot: object }>[] = [];
    const results = queueChildDeliveries(
      [
        {
          childInstance: "EBESExQVFhcYGRobHB0eHw",
          envelope: Object.freeze({ body: {}, signature: "A".repeat(43) }),
          parameterHash: "A".repeat(43),
        },
        {
          childInstance: "ICEiIyQlJicoKSorLC0uLw",
          envelope: Object.freeze({ body: {}, signature: "A".repeat(43) }),
          parameterHash: "B".repeat(43),
        },
      ],
      parentSnapshot,
      {
        find(instanceId) {
          if (instanceId !== "EBESExQVFhcYGRobHB0eHw") return null;
          return {
            active: () => true,
            instanceId,
            queueParamsChanged(_envelope, pairedParentSnapshot, hash) {
              queued.push(Object.freeze({ hash, parentSnapshot: pairedParentSnapshot }));
              return true;
            },
          };
        },
      },
    );
    expect(results).toEqual([
      { childInstance: "EBESExQVFhcYGRobHB0eHw", disposition: "queued" },
      { childInstance: "ICEiIyQlJicoKSorLC0uLw", disposition: "missing" },
    ]);
    expect(queued).toEqual([{ hash: "A".repeat(43), parentSnapshot }]);
  });

  it("contains one child construction failure and continues the remaining deliveries", () => {
    const parentSnapshot = Object.freeze({ body: Object.freeze({ revision: "8" }) });
    const queued: string[] = [];
    const deliveries = [
      {
        childInstance: "EBESExQVFhcYGRobHB0eHw",
        envelope: Object.freeze({ body: {}, signature: "A".repeat(43) }),
        parameterHash: "A".repeat(43),
      },
      {
        childInstance: "ICEiIyQlJicoKSorLC0uLw",
        envelope: Object.freeze({ body: {}, signature: "B".repeat(43) }),
        parameterHash: "B".repeat(43),
      },
    ];

    const results = queueChildDeliveries(deliveries, parentSnapshot, {
      find(instanceId) {
        return {
          active: () => true,
          instanceId,
          queueParamsChanged() {
            if (instanceId === deliveries[0]?.childInstance) throw new Error("intent_too_large");
            queued.push(instanceId);
            return true;
          },
        };
      },
    });

    expect(results).toEqual([
      { childInstance: deliveries[0]?.childInstance, disposition: "rejected" },
      { childInstance: deliveries[1]?.childInstance, disposition: "queued" },
    ]);
    expect(queued).toEqual([deliveries[1]?.childInstance]);
  });

  it("reflects query and fragment changes only on the current origin and path", () => {
    const current = new URL("https://example.test/catalog?page=1");
    const replaced: string[] = [];
    expect(
      applyUrlReflection(current, "/catalog?page=2#results", (target) => {
        replaced.push(target.href);
      }).href,
    ).toBe("https://example.test/catalog?page=2#results");
    expect(replaced).toEqual(["https://example.test/catalog?page=2#results"]);
    expect(() => reflectedUrl(current, "https://evil.test/catalog")).toThrow(UrlReflectionError);
    expect(() => reflectedUrl(current, "/other?page=2")).toThrow(UrlReflectionError);
    expect(() => reflectedUrl(current, "https://user:secret@example.test/catalog")).toThrow(
      UrlReflectionError,
    );
  });

  it("dispatches events before awaiting effects and preserves server order", async () => {
    const trace: string[] = [];
    const emissions = [
      Object.freeze({ name: "one", payload: null }),
      Object.freeze({ name: "two", payload: null }),
    ];
    dispatchValidatedEvents(emissions, { dispatch: ({ name }) => trace.push(`event:${name}`) });
    await runValidatedEffects(emissions, {
      effect: ({ name }) => {
        trace.push(`effect:${name}`);
        return Promise.resolve();
      },
    });
    expect(trace).toEqual(["event:one", "event:two", "effect:one", "effect:two"]);
  });
});
