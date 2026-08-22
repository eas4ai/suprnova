import { describe, expect, it } from "vitest";

import { queueChildDeliveries } from "../src/application/children.js";
import { dispatchValidatedEvents, runValidatedEffects } from "../src/application/emissions.js";
import { applyUrlReflection, reflectedUrl, UrlReflectionError } from "../src/application/url.js";

describe("post-commit application helpers", () => {
  it("queues signed child deliveries independently without rolling back the parent", () => {
    const queued: string[] = [];
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
      {
        find(instanceId) {
          if (instanceId !== "EBESExQVFhcYGRobHB0eHw") return null;
          return {
            active: () => true,
            instanceId,
            queueParamsChanged(_envelope, hash) {
              queued.push(hash);
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
    expect(queued).toEqual(["A".repeat(43)]);
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
