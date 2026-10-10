import { describe, expect, it } from "vitest";

import { ResponseApplicationMachine, type ApplicationPorts } from "../src/application/machine.js";
import type { ApplicationEpoch } from "../src/application/recovery.js";
import type { BrowserIslandAuthority, ResponseRequestAuthority } from "../src/application/types.js";
import { parseUpdateResponse } from "../src/protocol.js";

const CORRELATION = "EBESExQVFhcYGRobHB0eHw";
const INSTANCE = "MDEyMzQ1Njc4OTo7PD0-Pw";
const SIGNATURE = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

function authority(): BrowserIslandAuthority {
  return Object.freeze({
    active: true,
    component: "catalog.search",
    connectionEpoch: 1,
    documentKey: "primary",
    instanceId: INSTANCE,
    revision: 7n,
    slot: "search-slot",
    snapshotForm: "instance",
  });
}

function request(): ResponseRequestAuthority {
  return Object.freeze({
    applicationDisposition: "accepted",
    baseRevision: 7n,
    connectionEpoch: 1,
    correlationId: CORRELATION,
    protocol: 2,
    promotion: false,
  });
}

function response(input: Readonly<Record<string, unknown>> = {}) {
  return parseUpdateResponse(
    JSON.stringify({
      accepted_revision: "8",
      child_deliveries: [],
      correlation_id: CORRELATION,
      effects: [{ name: "toast", payload: {} }],
      events: [{ name: "saved", payload: {} }],
      extensions: {},
      outcome: "accepted",
      protocol_version: 2,
      render: { html: "<div data-suprnova-live-island>Saved</div>", kind: "html" },
      snapshot: {
        body: {
          component: { name: "catalog.search" },
          form: "instance",
          instance_id: INSTANCE,
          revision: "8",
          slot: "search-slot",
        },
        signature: SIGNATURE,
      },
      url_intent: { kind: "reflected", target: "/catalog?page=2" },
      validation: {},
      ...input,
    }),
  );
}

function ports(
  trace: string[],
  failMorph = false,
  failReconcile = false,
  failEffects = false,
  current = true,
  failChildQueue = false,
): ApplicationPorts<string> {
  const epoch = Object.freeze({
    acceptedRevision: 8n,
    connectionEpoch: 1,
    epoch: 1,
  }) satisfies ApplicationEpoch;
  return {
    applicationCurrent: () => current,
    beginApplication: () => epoch,
    commit: () => trace.push("commit_snapshot_and_revision"),
    completeApplication: () => undefined,
    dispatchEvents: () => trace.push("dispatch_events"),
    morph: () => {
      trace.push("morph");
      if (failMorph) throw new Error("morph_failed");
    },
    navigate: () => trace.push("navigate"),
    postCommitFailure: () => trace.push("post_commit_failure"),
    preflight: () => {
      trace.push("preflight_morph");
      return "prepared";
    },
    queueChildren: () => {
      trace.push("queue_child_deliveries");
      if (failChildQueue) throw new Error("child_queue_failed");
    },
    reconcile: () => {
      trace.push("reconcile_models_and_validation");
      if (failReconcile) throw new Error("reconcile_failed");
    },
    reflectUrl: () => trace.push("reflect_url"),
    recover: () => {
      trace.push("request_fresh_render_without_replay");
      return Object.freeze({ disposition: "request_fresh_render" });
    },
    requestFreshIsland: () => trace.push("request_fresh_island"),
    rollbackCommit: () => trace.push("rollback_commit"),
    restoreFocus: () => trace.push("restore_focus"),
    retainDom: () => trace.push("retain_dom"),
    runEffects: () => {
      trace.push("run_registered_effects");
      return failEffects ? Promise.reject(new Error("effect_failed")) : Promise.resolve();
    },
    settleFeedback: () => trace.push("settle_feedback"),
    stopLive: () => trace.push("stop_live"),
    validateNoRender: () => trace.push("validate_no_render"),
  };
}

describe("response application machine", () => {
  it("commits only after a successful morph and executes the locked order", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace)).apply(
      response({
        child_deliveries: [
          {
            child_instance: "ICEiIyQlJicoKSorLC0uLw",
            envelope: {},
            parameter_hash: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
          },
        ],
      }),
      authority(),
      request(),
    );
    expect(result).toEqual({ disposition: "committed" });
    expect(trace).toEqual([
      "preflight_morph",
      "morph",
      "commit_snapshot_and_revision",
      "reconcile_models_and_validation",
      "restore_focus",
      "queue_child_deliveries",
      "reflect_url",
      "dispatch_events",
      "run_registered_effects",
      "settle_feedback",
    ]);
  });

  it("preserves accepted browser state and requests a fresh render when morph fails", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace, true)).apply(
      response({ child_deliveries: [], url_intent: null }),
      authority(),
      request(),
    );
    expect(result).toEqual({ disposition: "fresh_render" });
    expect(trace).toEqual(["preflight_morph", "morph", "request_fresh_render_without_replay"]);
  });

  it("rolls back committed projections before recovering an application-order failure", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace, false, true)).apply(
      response({ child_deliveries: [], url_intent: null }),
      authority(),
      request(),
    );
    expect(result).toEqual({ disposition: "fresh_render" });
    expect(trace).toEqual([
      "preflight_morph",
      "morph",
      "commit_snapshot_and_revision",
      "reconcile_models_and_validation",
      "rollback_commit",
      "request_fresh_render_without_replay",
    ]);
  });

  it("contains effect rejection without rolling back an already committed server outcome", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace, false, false, true)).apply(
      response({ child_deliveries: [], url_intent: null }),
      authority(),
      request(),
    );
    expect(result).toEqual({ disposition: "committed" });
    expect(trace).toContain("post_commit_failure");
    expect(trace).not.toContain("rollback_commit");
    expect(trace).not.toContain("request_fresh_render_without_replay");
  });

  it("contains child scheduling failure after the parent commit without parent recovery", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(
      ports(trace, false, false, false, true, true),
    ).apply(
      response({
        child_deliveries: [
          {
            child_instance: "ICEiIyQlJicoKSorLC0uLw",
            envelope: {},
            parameter_hash: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
          },
        ],
      }),
      authority(),
      request(),
    );

    expect(result).toEqual({ disposition: "committed" });
    expect(trace).toEqual([
      "preflight_morph",
      "morph",
      "commit_snapshot_and_revision",
      "reconcile_models_and_validation",
      "restore_focus",
      "queue_child_deliveries",
      "post_commit_failure",
      "reflect_url",
      "dispatch_events",
      "run_registered_effects",
      "settle_feedback",
    ]);
  });

  it("ignores a late response whose application epoch was invalidated during morph", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(
      ports(trace, false, false, false, false),
    ).apply(response({ child_deliveries: [], url_intent: null }), authority(), request());
    expect(result).toEqual({ disposition: "stale_application" });
    expect(trace).toEqual(["preflight_morph", "morph"]);
  });

  it("navigates immediately without applying any in-page response state", async () => {
    const trace: string[] = [];
    const navigation = parseUpdateResponse(
      JSON.stringify({
        child_deliveries: [],
        correlation_id: CORRELATION,
        effects: [],
        events: [],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        url_intent: { kind: "navigated", target: "/next" },
        validation: {},
      }),
    );
    expect(
      await new ResponseApplicationMachine(ports(trace)).apply(navigation, authority(), request()),
    ).toEqual({ disposition: "navigated" });
    expect(trace).toEqual(["navigate"]);
  });

  it("never lets a terminal navigation escape a retired island", async () => {
    const trace: string[] = [];
    const navigation = parseUpdateResponse(
      JSON.stringify({
        child_deliveries: [],
        correlation_id: CORRELATION,
        effects: [],
        events: [],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        url_intent: { kind: "navigated", target: "/next" },
        validation: {},
      }),
    );
    expect(
      await new ResponseApplicationMachine(ports(trace)).apply(
        navigation,
        Object.freeze({ ...authority(), active: false }),
        request(),
      ),
    ).toEqual({ disposition: "retired" });
    expect(trace).toEqual([]);
  });

  it("rejects ineligible responses without invoking an application port", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace)).apply(
      response(),
      authority(),
      { ...request(), applicationDisposition: "out_of_order" },
    );
    expect(result).toEqual({ disposition: "application_slot" });
    expect(trace).toEqual([]);
  });

  it("validates no-render before commit", async () => {
    const trace: string[] = [];
    const result = await new ResponseApplicationMachine(ports(trace)).apply(
      response({ render: { kind: "no_render" }, url_intent: null }),
      authority(),
      request(),
    );
    expect(result).toEqual({ disposition: "committed" });
    expect(trace).toEqual([
      "validate_no_render",
      "commit_snapshot_and_revision",
      "reconcile_models_and_validation",
      "restore_focus",
      "dispatch_events",
      "run_registered_effects",
      "settle_feedback",
    ]);
  });

  it.each([
    ["rejected", "retain_dom", "rejected", ["retain_dom", "settle_feedback"]],
    [
      "refresh_required",
      "refresh_island",
      "fresh_island",
      ["retain_dom", "request_fresh_island", "settle_feedback"],
    ],
    ["fatal", "stop", "stopped", ["retain_dom", "stop_live", "settle_feedback"]],
  ] as const)(
    "applies %s recovery without touching accepted state",
    async (outcome, recovery, disposition, expected) => {
      const trace: string[] = [];
      const rejected = parseUpdateResponse(
        JSON.stringify({
          child_deliveries: [],
          correlation_id: CORRELATION,
          effects: [],
          error: { category: "internal", detail: "operation_rejected", recovery },
          events: [],
          extensions: {},
          outcome,
          protocol_version: 2,
          url_intent: null,
          validation: {},
        }),
      );
      expect(
        await new ResponseApplicationMachine(ports(trace)).apply(rejected, authority(), request()),
      ).toEqual({ disposition });
      expect(trace).toEqual(expected);
    },
  );
});
