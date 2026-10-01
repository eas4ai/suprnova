import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import {
  MAX_EVENTS,
  MAX_MODEL_PROPOSALS,
  MAX_OPERATIONS,
  MAX_VALIDATION_ENTRIES,
  ProtocolValidationError,
  validateUpdateRequest,
  validateUpdateResponse,
} from "../src/protocol.js";
import { refusedRequestDiagnostic } from "../src/transport/fetch.js";
import { LiveTransportError } from "../src/transport/state.js";

// LIVE-028: the browser admits the message counts the framework's server
// configures in ProtocolLimits (128 each). The browser had refused more than
// eight proposals, operations and events and more than sixteen validation
// entries, so a ten-field form never submitted against a server that
// accepted it.

type Json = Record<string, JsonValue>;

function fixture(id: string): Json {
  const fixtures = JSON.parse(
    readFileSync(new URL("../../fixtures/v2/protocol-success.json", import.meta.url), "utf8"),
  ) as { cases: { id: string; encoded: string }[] };
  const found = fixtures.cases.find((candidate) => candidate.id === id);
  if (found === undefined) throw new Error(`missing fixture ${id}`);
  return JSON.parse(found.encoded) as Json;
}

function submitRequest(fields: number): string {
  const request = fixture("instance-action-request");
  const names = Array.from({ length: fields }, (_, index) => `field_${String(index)}`);
  request["model_proposals"] = Object.fromEntries(names.map((name) => [name, "value"]));
  request["operations"] = [
    ...names.map((field) => ({ field, kind: "sync_model" })),
    { arguments: {}, kind: "invoke_action", name: "search" },
  ];
  return canonicalize(request);
}

function responseWith(validation: number, events: number): string {
  const response = fixture("reflected-url-response");
  response["validation"] = Object.fromEntries(
    Array.from({ length: validation }, (_, index) => [`field_${String(index)}`, "Required"]),
  );
  response["events"] = Array.from({ length: events }, (_, index) => ({
    name: `event_${String(index)}`,
    payload: {},
  }));
  return canonicalize(response);
}

function refusal(action: () => void): string | null {
  try {
    action();
    return null;
  } catch (error: unknown) {
    return error instanceof ProtocolValidationError ? error.code : String(error);
  }
}

describe("LIVE-028: the browser admits the framework's protocol counts", () => {
  it("bounds every count at the framework's 128", () => {
    expect([MAX_MODEL_PROPOSALS, MAX_OPERATIONS, MAX_VALIDATION_ENTRIES, MAX_EVENTS]).toEqual([
      128, 128, 128, 128,
    ]);
  });

  it("sends a submit of ten fields, and of 127, and refuses 128 fields plus the action", () => {
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(10));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(127));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(128));
      }),
    ).toBe("too_many_operations");
  });

  it("accepts a response with seventeen validation entries and nine events, and refuses 129", () => {
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(17, 9));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(128, 128));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(129, 0));
      }),
    ).toBe("protocol_too_many_entries");
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(0, 129));
      }),
    ).toBe("protocol_too_many_entries");
  });
});

// LIVE-030: a request the builder refuses for a protocol bound never left the
// browser; it is reported as a resource limit, while a real network failure
// keeps its transport diagnostic.
describe("LIVE-030: an oversized request is a resource limit", () => {
  it("classifies a bound refusal as resource_limit and nothing else", () => {
    for (const code of [
      "too_many_model_proposals",
      "too_many_operations",
      "protocol_too_many_entries",
    ]) {
      expect(refusedRequestDiagnostic(new ProtocolValidationError(code))).toEqual({
        code: "resource_limit",
        detailCode: "resource_exhausted",
        phase: "transport",
        severity: "error",
      });
    }
    expect(
      refusedRequestDiagnostic(new ProtocolValidationError("invalid_protocol_envelope")),
    ).toBeNull();
    expect(refusedRequestDiagnostic(new LiveTransportError("network"))).toBeNull();
    expect(refusedRequestDiagnostic(new TypeError("raw network detail"))).toBeNull();
  });
});
