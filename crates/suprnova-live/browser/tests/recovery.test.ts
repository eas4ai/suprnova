import { describe, expect, it } from "vitest";

import { ApplicationRecovery } from "../src/application/recovery.js";
import { projectFeedback, type FeedbackWorkRecord } from "../src/feedback/state.js";

const identity = Object.freeze({ acceptedRevision: 8n, connectionEpoch: 3 });

function completedAccepted(): FeedbackWorkRecord {
  return Object.freeze({
    actions: Object.freeze(["save"]),
    disposition: "accepted",
    fields: Object.freeze([]),
    intentId: "intent-1",
    offline: false,
    phase: "completed",
    retrying: false,
  });
}

describe("fresh-render application recovery", () => {
  it("invalidates a failed accepted application and requests one action-free fresh render", () => {
    const recovery = new ApplicationRecovery();
    const original = recovery.begin(identity);

    expect(recovery.fail(original)).toEqual({ disposition: "request_fresh_render" });
    expect(recovery.state()).toBe("fresh_render_pending");
    expect(recovery.current(original)).toBe(false);
    expect(recovery.freshRenderOperation()).toEqual({
      childParameters: [],
      kind: "fresh_render",
      modelProposals: [],
      originalAction: null,
    });
  });

  it("disconnects only the island when recovery application fails a second time", () => {
    const recovery = new ApplicationRecovery();
    const original = recovery.begin(identity);
    expect(recovery.fail(original).disposition).toBe("request_fresh_render");

    const fresh = recovery.begin(identity);
    expect(recovery.fail(fresh)).toEqual({ disposition: "disconnect_island" });
    expect(recovery.state()).toBe("disconnected");
    expect(recovery.begin(identity)).toBeNull();
  });

  it("ignores late original callbacks after a successful recovery epoch", () => {
    const recovery = new ApplicationRecovery();
    const original = recovery.begin(identity);
    recovery.fail(original);
    const fresh = recovery.begin(identity);

    expect(recovery.succeed(fresh)).toBe(true);
    expect(recovery.state()).toBe("none");
    expect(recovery.fail(original)).toEqual({ disposition: "ignored" });
  });

  it("resets the bounded attempt only after success or a new connection epoch", () => {
    const recovery = new ApplicationRecovery();
    const original = recovery.begin(identity);
    recovery.fail(original);
    const fresh = recovery.begin(identity);
    expect(recovery.succeed(fresh)).toBe(true);

    const later = recovery.begin({ acceptedRevision: 9n, connectionEpoch: 3 });
    expect(recovery.fail(later).disposition).toBe("request_fresh_render");

    const reconnected = recovery.begin({ acceptedRevision: 9n, connectionEpoch: 4 });
    expect(recovery.fail(reconnected).disposition).toBe("request_fresh_render");
  });

  it("projects recovery as interrupted/recovering and suppresses stale success/loading", () => {
    const snapshot = projectFeedback(
      [completedAccepted()],
      null,
      Object.freeze({ kind: "island", value: "primary" }),
      "fresh_render_pending",
    );
    expect([...snapshot.states]).toEqual(["interrupted"]);
    expect(snapshot.recovery).toBe("fresh_render_pending");
    expect(snapshot.states.has("success")).toBe(false);
    expect(snapshot.states.has("loading")).toBe(false);
  });
});
