import { describe, expect, it } from "vitest";

import { evaluateResponseEligibility } from "../src/application/eligibility.js";
import type { BrowserIslandAuthority, ResponseRequestAuthority } from "../src/application/types.js";
import { parseUpdateResponse, ProtocolValidationError } from "../src/protocol.js";

const CORRELATION = "EBESExQVFhcYGRobHB0eHw";
const INSTANCE = "MDEyMzQ1Njc4OTo7PD0-Pw";
const SIGNATURE = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

function snapshot(revision = "8", instance = INSTANCE): Readonly<Record<string, unknown>> {
  return {
    body: {
      component: { name: "catalog.search" },
      form: "instance",
      instance_id: instance,
      revision,
      slot: "search-slot",
    },
    signature: SIGNATURE,
  };
}

function committed(overrides: Readonly<Record<string, unknown>> = {}): string {
  return JSON.stringify({
    accepted_revision: "8",
    child_deliveries: [],
    correlation_id: CORRELATION,
    effects: [{ name: "toast", payload: { message: "Saved" } }],
    events: [{ name: "saved", payload: { id: 7 } }],
    extensions: {},
    outcome: "accepted",
    protocol_version: 2,
    render: { kind: "html", html: "<div>Saved</div>" },
    snapshot: snapshot(),
    url_intent: null,
    validation: {},
    ...overrides,
  });
}

function island(overrides: Partial<BrowserIslandAuthority> = {}): BrowserIslandAuthority {
  return Object.freeze({
    active: true,
    component: "catalog.search",
    connectionEpoch: 1,
    documentKey: "primary",
    instanceId: INSTANCE,
    revision: 7n,
    slot: "search-slot",
    snapshotForm: "instance",
    ...overrides,
  });
}

function request(overrides: Partial<ResponseRequestAuthority> = {}): ResponseRequestAuthority {
  return Object.freeze({
    applicationDisposition: "accepted",
    baseRevision: 7n,
    connectionEpoch: 1,
    correlationId: CORRELATION,
    protocol: 2,
    promotion: false,
    ...overrides,
  });
}

describe("typed response parsing", () => {
  it("returns immutable committed, navigation, and rejected response variants", () => {
    const parsed = parseUpdateResponse(committed());
    expect(parsed).toMatchObject({
      acceptedRevision: 8n,
      correlationId: CORRELATION,
      kind: "committed",
      outcome: "accepted",
      protocol: 2,
      render: { kind: "html", html: "<div>Saved</div>" },
    });
    expect(Object.isFrozen(parsed)).toBe(true);
    if (parsed.kind !== "committed") throw new Error("expected committed response");
    expect(Object.isFrozen(parsed.events)).toBe(true);
    expect(Object.isFrozen(parsed.events[0]?.payload)).toBe(true);
    expect(
      parseUpdateResponse(
        JSON.stringify({
          child_deliveries: [],
          correlation_id: CORRELATION,
          effects: [],
          events: [],
          extensions: {},
          outcome: "accepted",
          protocol_version: 2,
          url_intent: { kind: "navigated", target: "/catalog?page=2" },
          validation: {},
        }),
      ),
    ).toMatchObject({ kind: "navigation", target: "/catalog?page=2" });
    expect(
      parseUpdateResponse(
        JSON.stringify({
          child_deliveries: [],
          correlation_id: CORRELATION,
          effects: [],
          error: { category: "validation", detail: "invalid_identifier", recovery: "retain_dom" },
          events: [],
          extensions: {},
          outcome: "rejected",
          protocol_version: 2,
          url_intent: null,
          validation: { name: ["required"] },
        }),
      ),
    ).toMatchObject({ kind: "rejected", recovery: "retain_dom" });
  });

  it("retains the void compatibility validator and closes malformed emission shape", () => {
    const malformed = committed({ events: [{ name: "saved", payload: {}, raw: "forged" }] });
    expect(() => parseUpdateResponse(malformed)).toThrow(ProtocolValidationError);
  });
});

describe("response eligibility", () => {
  it("accepts one exact legal successor without mutating browser authority", () => {
    const current = island();
    const response = parseUpdateResponse(committed());
    expect(evaluateResponseEligibility(response, current, request())).toEqual({
      disposition: "accepted",
    });
    expect(current.revision).toBe(7n);
  });

  it.each([
    ["correlation", request({ correlationId: "MDEyMzQ1Njc4OTo7PD0-Pw" }), island()],
    ["protocol", request({ protocol: 1 }), island()],
    ["base_revision", request({ baseRevision: 6n }), island()],
    ["connection_epoch", request({ connectionEpoch: 2 }), island()],
    ["retired", request(), island({ active: false })],
    ["application_slot", request({ applicationDisposition: "out_of_order" }), island()],
    ["application_slot", request({ applicationDisposition: "duplicate" }), island()],
    ["application_slot", request({ applicationDisposition: "stale" }), island()],
    ["application_slot", request({ applicationDisposition: "canceled" }), island()],
    ["application_slot", request({ applicationDisposition: "superseded" }), island()],
  ] as const)("rejects %s mismatch before application", (disposition, authority, current) => {
    expect(
      evaluateResponseEligibility(parseUpdateResponse(committed()), current, authority),
    ).toEqual({ disposition });
  });

  it("rejects wrong successor revision, island identity, and seed-promotion form", () => {
    expect(
      evaluateResponseEligibility(
        parseUpdateResponse(committed({ accepted_revision: "9", snapshot: snapshot("9") })),
        island(),
        request(),
      ),
    ).toEqual({ disposition: "successor_revision" });
    expect(
      evaluateResponseEligibility(
        parseUpdateResponse(committed({ snapshot: snapshot("8", "ICEiIyQlJicoKSorLC0uLw") })),
        island(),
        request(),
      ),
    ).toEqual({ disposition: "island" });
    expect(
      evaluateResponseEligibility(
        parseUpdateResponse(committed()),
        island({ snapshotForm: "seed" }),
        request(),
      ),
    ).toEqual({ disposition: "snapshot_form" });
    expect(
      evaluateResponseEligibility(
        parseUpdateResponse(
          committed({
            snapshot: {
              body: {
                component: { name: "catalog.search" },
                form: "seed",
                slot: "search-slot",
              },
              signature: SIGNATURE,
            },
          }),
        ),
        island(),
        request(),
      ),
    ).toEqual({ disposition: "snapshot_form" });
  });
});
