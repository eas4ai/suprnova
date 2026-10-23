import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import { SERVER_DEFAULT_LIMITS, type LiveLimits } from "../src/limits.js";
import {
  ProtocolValidationError,
  validateUpdateRequest,
  validateUpdateResponse,
} from "../src/protocol.js";
import { refusedRequestDiagnostic } from "../src/transport/fetch.js";
import { LiveTransportError } from "../src/transport/state.js";

// LIVE-028: the browser admits the message counts the framework's server
// configures. The browser had refused more than eight proposals, operations
// and events and more than sixteen validation entries, so a ten-field form
// never submitted against a server that accepted it. Since 2026-10-04 the
// counts are the server's LIVE_MAX_REQUEST_ITEMS and LIVE_MAX_RESPONSE_ITEMS,
// read from the boot configuration, never a browser constant.

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

function limits(overrides: Partial<LiveLimits>): LiveLimits {
  return Object.freeze({ ...SERVER_DEFAULT_LIMITS, ...overrides });
}

describe("LIVE-028: the browser admits the framework's protocol counts", () => {
  it("defaults every count to the server's default, far above the old 128", () => {
    expect(SERVER_DEFAULT_LIMITS.maxRequestItems).toBe(65_536);
    expect(SERVER_DEFAULT_LIMITS.maxResponseItems).toBe(65_536);
  });

  it("sends a submit of ten fields and of 1,000, and refuses one over the configured count", () => {
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(10));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(1_000));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(127), limits({ maxRequestItems: 128 }));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateRequest(submitRequest(128), limits({ maxRequestItems: 128 }));
      }),
    ).toBe("too_many_operations");
  });

  it("names the request item setting when a submit is over it", () => {
    let message = "";
    try {
      validateUpdateRequest(submitRequest(128), limits({ maxRequestItems: 128 }));
    } catch (error: unknown) {
      message = (error as Error).message;
    }
    expect(message).toContain("request item count limit exceeded (operations)");
    expect(message).toContain("measured 129 items, configured 128 items");
    expect(message).toContain("LIVE_MAX_REQUEST_ITEMS");
  });

  it("accepts a response with 1,000 validation entries and events, and refuses one over", () => {
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(17, 9));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(1_000, 1_000));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(129, 0), limits({ maxResponseItems: 128 }));
      }),
    ).toBe("protocol_too_many_entries");
    expect(
      refusal(() => {
        validateUpdateResponse(responseWith(0, 129), limits({ maxResponseItems: 128 }));
      }),
    ).toBe("protocol_too_many_entries");
  });

  it("refuses a request over the configured byte limit, naming LIVE_MAX_REQUEST_BYTES", () => {
    const text = submitRequest(10);
    let error: unknown = null;
    try {
      validateUpdateRequest(text, limits({ maxRequestBytes: 64 }));
    } catch (caught: unknown) {
      error = caught;
    }
    expect(error).toBeInstanceOf(ProtocolValidationError);
    expect((error as ProtocolValidationError).code).toBe("protocol_input_too_large");
    expect((error as Error).message).toContain(
      `measured ${String(new TextEncoder().encode(text).byteLength)} bytes, configured 64 bytes`,
    );
    expect((error as Error).message).toContain("LIVE_MAX_REQUEST_BYTES");
  });
});

// LIVE-030: a request the builder refuses for a protocol bound never left the
// browser; it is reported as a resource limit, while a real network failure
// keeps its transport diagnostic.
describe("LIVE-030: an oversized request is a resource limit", () => {
  it("classifies a bound refusal as resource_limit and nothing else", () => {
    expect(
      refusedRequestDiagnostic(
        (() => {
          try {
            validateUpdateRequest(submitRequest(10), limits({ maxRequestBytes: 64 }));
          } catch (error: unknown) {
            return error;
          }
          return null;
        })(),
      ),
    ).toMatchObject({ code: "resource_limit" });
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
