import { describe, expect, it } from "vitest";

import { FeedbackAnnouncer } from "../src/feedback/announcer.js";
import {
  projectFeedback,
  type FeedbackScope,
  type FeedbackWorkRecord,
} from "../src/feedback/state.js";
import { ModelState } from "../src/models/state.js";
import type { IntentSource } from "../src/scheduler/intent.js";
import { ServerIntent } from "../src/scheduler/intent.js";
import { IslandScheduler } from "../src/scheduler/scheduler.js";

function work(overrides: Partial<FeedbackWorkRecord> = {}): FeedbackWorkRecord {
  return Object.freeze({
    actions: Object.freeze(["save"]),
    disposition: null,
    fields: Object.freeze(["name"]),
    intentId: "intent-1",
    offline: false,
    phase: "pending",
    retrying: false,
    ...overrides,
  });
}

function states(
  records: readonly FeedbackWorkRecord[],
  model: ModelState | null,
  scope: FeedbackScope,
): readonly string[] {
  return [...projectFeedback(records, model, scope).states].sort();
}

describe("truthful feedback projection", () => {
  it("projects idle and dirty from model authority without treating a proposal as success", () => {
    const model = new ModelState();
    model.register("name", "Ada");
    const scope = Object.freeze({ kind: "field", value: "name" }) satisfies FeedbackScope;
    expect(states([], model, scope)).toEqual(["idle"]);

    model.propose("name", "Grace");
    expect(states([], model, scope)).toEqual(["dirty"]);
    expect(states([], model, scope)).not.toContain("success");
  });

  it("distinguishes queued, loading, validating, retrying, and offline work by scope", () => {
    const model = new ModelState();
    model.register("name", "Ada");
    model.markInFlight("name", "intent-1");

    expect(states([work()], model, Object.freeze({ kind: "action", value: "save" }))).toEqual([
      "queued",
    ]);
    expect(
      states(
        [work({ phase: "in_flight" })],
        model,
        Object.freeze({ kind: "field", value: "name" }),
      ),
    ).toEqual(["loading", "validating"]);
    expect(
      states(
        [work({ offline: true, phase: "in_flight", retrying: true })],
        model,
        Object.freeze({ kind: "island", value: "primary" }),
      ),
    ).toEqual(["loading", "offline", "retrying"]);
  });

  it("projects terminal success, interruption, validation failure, and transport failure distinctly", () => {
    const action = Object.freeze({ kind: "action", value: "save" }) satisfies FeedbackScope;
    expect(states([work({ disposition: "accepted", phase: "completed" })], null, action)).toEqual([
      "success",
    ]);
    expect(states([work({ disposition: "canceled", phase: "completed" })], null, action)).toEqual([
      "interrupted",
    ]);
    expect(states([work({ disposition: "rejected", phase: "completed" })], null, action)).toEqual([
      "error",
    ]);
    expect(
      states(
        [
          work({
            disposition: "rejected",
            offline: true,
            phase: "completed",
            retrying: false,
          }),
        ],
        null,
        action,
      ),
    ).toEqual(["error", "offline"]);

    const model = new ModelState();
    model.register("name", "Ada");
    model.setValidation("name", [{ message: "required" }]);
    expect(states([], model, Object.freeze({ kind: "field", value: "name" }))).toEqual(["error"]);
  });

  it("combines compatible aggregate work but never leaks an unrelated target", () => {
    const records = [
      work(),
      work({
        actions: Object.freeze(["delete"]),
        fields: Object.freeze([]),
        intentId: "intent-2",
        phase: "in_flight",
      }),
    ];
    expect(states(records, null, Object.freeze({ kind: "island", value: "primary" }))).toEqual([
      "loading",
      "queued",
    ]);
    expect(states(records, null, Object.freeze({ kind: "action", value: "archive" }))).toEqual([
      "idle",
    ]);
  });
});

describe("authoritative feedback sources", () => {
  it("publishes bounded scheduler transitions with semantic action and field scope", () => {
    const scheduler = new IslandScheduler({
      maxCompleted: 8,
      maxParallel: 1,
      maxQueued: 4,
      maxRecoveries: 2,
    });
    let notifications = 0;
    const unsubscribe = scheduler.subscribeFeedback(() => {
      notifications += 1;
    });
    const intent = new ServerIntent(
      Object.freeze({ eventType: "submit" }) as unknown as IntentSource,
      [
        Object.freeze({ field: "name", kind: "sync_model" }),
        Object.freeze({ arguments: Object.freeze({}), kind: "invoke_action", name: "save" }),
      ],
      null,
      Object.freeze({ name: "Grace" }),
      Object.freeze({ name: 1n }),
    );
    const scheduled = scheduler.schedule(intent);
    expect(scheduled.ticket).toBeDefined();
    if (scheduled.ticket === undefined) throw new Error("missing scheduler ticket");

    expect(scheduler.feedback()).toMatchObject([
      {
        actions: ["save"],
        disposition: null,
        fields: ["name"],
        intentId: "1",
        phase: "pending",
      },
    ]);
    scheduler.start(scheduled.ticket);
    expect(
      scheduler.setTransportFeedback(scheduled.ticket, { offline: true, retrying: true }),
    ).toBe("accepted");
    expect(scheduler.feedback()[0]).toMatchObject({
      offline: true,
      phase: "in_flight",
      retrying: true,
    });
    scheduler.finish(scheduled.ticket, "rejected");
    expect(scheduler.feedback()[0]).toMatchObject({
      disposition: "rejected",
      offline: true,
      phase: "completed",
    });
    expect(notifications).toBeGreaterThanOrEqual(4);
    unsubscribe();
  });

  it("notifies model observers only when authoritative model state changes", () => {
    const model = new ModelState();
    let notifications = 0;
    const unsubscribe = model.subscribe(() => {
      notifications += 1;
    });
    model.register("name", "Ada");
    model.propose("name", "Ada");
    model.propose("name", "Grace");
    model.markInFlight("name", "intent-1");
    model.clearInFlight("name", "intent-1");
    model.reconcile("name", "Ada", 0n, []);
    expect(notifications).toBe(4);
    unsubscribe();
    model.setValidation("name", [{ message: "required" }]);
    expect(notifications).toBe(4);
  });
});

describe("feedback announcements", () => {
  it("coalesces one equivalent scope transition while keeping distinct failure messages", () => {
    const messages: string[] = [];
    const announcer = new FeedbackAnnouncer((announcement) => {
      messages.push(`${announcement.politeness}:${announcement.message}`);
    });

    expect(announcer.announce("field:name", "validation", "intent-1", "polite")).toBe(true);
    expect(announcer.announce("field:name", "validation", "intent-1", "polite")).toBe(false);
    expect(announcer.announce("field:name", "retry", "intent-1", "polite")).toBe(true);
    expect(announcer.announce("field:name", "failure", "intent-1", "assertive")).toBe(true);
    expect(announcer.announce("field:name", "failure", "intent-2", "assertive")).toBe(true);

    expect(messages).toEqual([
      "polite:Validation failed",
      "polite:Retrying",
      "assertive:Request failed",
      "assertive:Request failed",
    ]);
  });

  it("uses a bounded time window so a later meaningful reconnect cycle can announce again", () => {
    const messages: string[] = [];
    let now = 1_000;
    const announcer = new FeedbackAnnouncer(
      (announcement) => {
        messages.push(`${announcement.politeness}:${announcement.message}`);
      },
      { maximumPerWindow: 3, now: () => now, windowMs: 1_000 },
    );

    expect(announcer.announce("stream:orders", "stream_degraded", "degraded", "polite")).toBe(true);
    expect(announcer.announce("stream:orders", "stream_degraded", "degraded", "polite")).toBe(
      false,
    );
    expect(
      announcer.announce("stream:orders", "stream_reconnecting", "reconnecting", "polite"),
    ).toBe(true);
    expect(announcer.announce("stream:orders", "stream_current", "current", "polite")).toBe(true);
    expect(announcer.announce("stream:orders", "stream_closed", "closed", "polite")).toBe(false);

    now += 1_001;
    expect(announcer.announce("stream:orders", "stream_degraded", "degraded", "polite")).toBe(true);
    expect(messages).toEqual([
      "polite:Updates degraded",
      "polite:Reconnecting to updates",
      "polite:Updates current",
      "polite:Updates degraded",
    ]);
  });
});
