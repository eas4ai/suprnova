import { readFileSync } from "node:fs";

import { describe, expect, it, vi } from "vitest";

import { decodeAsyncEnvelope } from "../src/async-updates/envelope.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import { canonicalize, parseCanonicalJson, type JsonValue } from "../src/canonical.js";
import { breachOf, limitBreach, SERVER_DEFAULT_LIMITS, type LiveLimits } from "../src/limits.js";
import { IdiomorphAdapter } from "../src/morph/idiomorph.js";
import { morphLimitsFrom } from "../src/morph/limits.js";
import { preflightIslandMorph } from "../src/morph/preflight.js";
import { decodeSnapshotPublicView } from "../src/protocol/snapshot-view.js";
import { ProtocolValidationError, validateUpdateResponse } from "../src/protocol.js";
import { CONFIG_ELEMENT_ID, parseRuntimeConfig } from "../src/runtime/config.js";
import { CoreRuntimeDiagnostics } from "../src/runtime/diagnostics.js";
import { asElement, element, morphFixture, text } from "./support/morph-dom.js";

// The 2026-10-04 ruling: no arbitrary small caps. Every limit is the server's
// configured value, defaults are sized for modern pages, and the browser never
// refuses what the server's configuration allows.

const MIB = 1024 * 1024;

vi.mock("idiomorph", () => ({ Idiomorph: { morph: vi.fn() } }));

type Json = Record<string, JsonValue>;

function protocolFixture(id: string): Json {
  const fixtures = JSON.parse(
    readFileSync(new URL("../../fixtures/v2/protocol-success.json", import.meta.url), "utf8"),
  ) as { cases: { id: string; encoded: string }[] };
  const found = fixtures.cases.find((candidate) => candidate.id === id);
  if (found === undefined) throw new Error(`missing fixture ${id}`);
  return JSON.parse(found.encoded) as Json;
}

function responseWithHtml(bytes: number): string {
  const response = protocolFixture("child-delivery-response");
  const inner = "x".repeat(bytes - "<section></section>".length);
  response["render"] = { html: `<section>${inner}</section>`, kind: "html" };
  return canonicalize(response);
}

function limits(overrides: Partial<LiveLimits>): LiveLimits {
  return Object.freeze({ ...SERVER_DEFAULT_LIMITS, ...overrides });
}

function refusal(action: () => void): unknown {
  try {
    action();
    return null;
  } catch (error: unknown) {
    return error;
  }
}

describe("island renders the server allows", () => {
  it("accepts a 2 MiB island render in the protocol validator", () => {
    expect(
      refusal(() => {
        validateUpdateResponse(responseWithHtml(2 * MIB));
      }),
    ).toBeNull();
    expect(
      refusal(() => {
        validateUpdateResponse(responseWithHtml(2 * MIB), SERVER_DEFAULT_LIMITS);
      }),
    ).toBeNull();
  });

  it("still refuses a render over the configured island HTML limit, naming the key", () => {
    const error = refusal(() => {
      validateUpdateResponse(responseWithHtml(4096), limits({ maxHtmlBytes: 2048 }));
    });
    expect(error).toBeInstanceOf(ProtocolValidationError);
    expect((error as ProtocolValidationError).code).toBe("protocol_input_too_large");
    const message = (error as Error).message;
    expect(message).toContain("island HTML size");
    expect(message).toContain("measured 4096 bytes");
    expect(message).toContain("configured 2048 bytes");
    expect(message).toContain("LIVE_MAX_HTML_BYTES");
  });

  it("morphs a 2 MiB render", () => {
    const fixture = morphFixture({
      replacementChildren: [],
      limits: morphLimitsFrom(SERVER_DEFAULT_LIMITS),
    });
    const plan = preflightIslandMorph({
      authority: fixture.authority,
      currentRoot: asElement(fixture.currentRoot),
      html: `<section>${"x".repeat(2 * MIB)}</section>`,
      limits: fixture.limits,
      parser: fixture.parser,
    });
    expect(plan.replacementRoot).toBeDefined();
  });

  it("morphs a render of 20,000 nodes, without a deadline by default", () => {
    const fixture = morphFixture({
      limits: morphLimitsFrom(SERVER_DEFAULT_LIMITS),
    });
    const rows = Array.from({ length: 10_000 }, (_, index) =>
      element(fixture.replacementDocument, "tr", {}, [
        text(fixture.replacementDocument, `row ${String(index)}`),
      ]),
    );
    for (const row of rows) fixture.replacementRoot.append(row);
    const plan = preflightIslandMorph({
      authority: fixture.authority,
      currentRoot: asElement(fixture.currentRoot),
      html: "<section></section>",
      limits: fixture.limits,
      parser: fixture.parser,
    });
    let now = 0;
    const adapter = new IdiomorphAdapter(() => {
      now += 1_000;
      return now;
    });
    expect(() => adapter.apply(plan, {})).not.toThrow();
  });

  it("refuses a render over the configured node limit with a message naming the key", () => {
    const fixture = morphFixture({
      limits: morphLimitsFrom(limits({ morphMaxNodes: 100 })),
    });
    for (let index = 0; index < 200; index += 1) {
      fixture.replacementRoot.append(element(fixture.replacementDocument, "td"));
    }
    const error = refusal(() =>
      preflightIslandMorph({
        authority: fixture.authority,
        currentRoot: asElement(fixture.currentRoot),
        html: "<section></section>",
        limits: fixture.limits,
        parser: fixture.parser,
      }),
    );
    const breach = breachOf(error);
    expect(breach).toMatchObject({
      configured: 100,
      key: "LIVE_MORPH_MAX_NODES",
      limit: "morphMaxNodes",
      measured: 101,
    });
    expect((error as Error).message).toContain("morph node count");
    expect((error as Error).message).toContain("LIVE_MORPH_MAX_NODES");
  });
});

describe("snapshots the server issues", () => {
  it("round-trips a snapshot with a 100 KiB string and 10,000 entries", () => {
    const fixtures = JSON.parse(
      readFileSync(new URL("../../fixtures/v1/snapshot-success.json", import.meta.url), "utf8"),
    ) as { cases: { id: string; encoded: Json }[] };
    const seed = fixtures.cases.find((candidate) => candidate.id === "seed-v1");
    if (seed === undefined) throw new Error("missing seed fixture");
    const envelope = structuredClone(seed.encoded) as { body: Json; signature: string };
    envelope.body["state"] = {
      notes: "n".repeat(100 * 1024),
      rows: Array.from({ length: 10_000 }, (_, index) => index),
    };
    const canonical = canonicalize(envelope);
    const encoded = Buffer.from(canonical, "utf8").toString("base64url");
    const view = decodeSnapshotPublicView(encoded);
    expect(view.component).toBe("catalog.search");
    expect(canonicalize(view.envelope as JsonValue)).toBe(canonical);
    expect(canonicalize(parseCanonicalJson(canonical))).toBe(canonical);
  });
});

describe("asynchronous envelopes the server sends", () => {
  it("accepts a browser event envelope over 32 KiB", () => {
    const membership = Object.freeze({
      authorization: Object.freeze({ kind: "session_cookie" as const }),
      baseline: Object.freeze({ epoch: 7n, sequence: 40n }),
      descriptorBinding: "descriptor-envelope-001",
      document: Object.freeze({
        authorizationScope: "document-envelope",
        origin: "https://app.example.test",
        transport: "sse" as const,
      }),
      events: Object.freeze([
        Object.freeze({
          cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
          maximum_fanout: 1,
          name: "orders.updated",
          order: "per_source_sequence" as const,
          payload_contract: "orders.updated.v1",
          schema: "json" as const,
          source: "stream" as const,
          targets: Object.freeze(["self"]),
          version: 1,
        }),
      ]),
      expiresAt: 10_000,
      fallbackPoll: Object.freeze({
        initial: "wait" as const,
        intervalMs: 30_000,
        jitterRatio: 0.2,
        visibility: "visible" as const,
      }),
      heartbeatTimeoutMs: 30_000,
      presentationSignals: Object.freeze([]),
      reconnect: Object.freeze({
        kind: "resume_or_refresh" as const,
        maximumAttempts: 4,
        maximumDelayMs: 30_000,
        minimumDelayMs: 250,
      }),
      stream: "orders",
      subscriptionId: "subscription-envelope-001",
    }) as AuthorizedLogicalSubscription;
    const encoded = canonicalize({
      payload: {
        event: "orders.updated",
        kind: "browser_event",
        payload: { rows: Array.from({ length: 4_000 }, (_, index) => `order-${String(index)}`) },
        schema_version: 1,
        target: "self",
      },
      position: { epoch: "7", sequence: "41" },
      protocol_version: 1,
      stream: "orders",
      subscription: "subscription-envelope-001",
    });
    expect(encoded.length).toBeGreaterThan(48 * 1024);
    expect(decodeAsyncEnvelope(encoded, membership).payload.kind).toBe("browser_event");
  });
});

describe("the boot configuration carries every limit", () => {
  const LIMITS_CONFIG = {
    max_html_bytes: 16 * MIB,
    max_json_depth: 32,
    max_json_entries: 1_000_000,
    max_request_bytes: 16 * MIB,
    max_request_items: 65_536,
    max_response_items: 65_536,
    morph_deadline_ms: 0,
    morph_max_attributes: 10_000_000,
    morph_max_attributes_per_element: 4_096,
    morph_max_depth: 512,
    morph_max_keys: 1_000_000,
    morph_max_nodes: 1_000_000,
  };

  function configDocument(config: Record<string, unknown>): Document {
    const node = {
      textContent: JSON.stringify(config),
      getAttribute: (name: string) => (name === "type" ? "application/json" : null),
    };
    return {
      baseURI: "https://app.example.test/page",
      querySelectorAll: (selector: string) =>
        selector === `[id="${CONFIG_ELEMENT_ID}"]` ? [node] : [],
    } as unknown as Document;
  }

  function config(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
      asset_identity: "limits-test-v1",
      credentials: "same-origin",
      endpoint: "/_suprnova/live",
      max_parallel_per_island: 1,
      max_queued_per_island: 8,
      max_response_bytes: 16 * MIB,
      protocol: { maximum: 2, minimum: 1 },
      request_timeout_ms: 60_000,
      runtime_contract_version: 1,
      ...LIMITS_CONFIG,
      ...overrides,
    };
  }

  it("reads the server's limits, including a 16 MiB and a 1 GiB response limit", () => {
    const parsed = parseRuntimeConfig(configDocument(config()));
    expect(parsed.limits).toEqual(SERVER_DEFAULT_LIMITS);
    const large = parseRuntimeConfig(
      configDocument(
        config({
          max_html_bytes: 1024 * MIB,
          max_request_bytes: 1024 * MIB,
          max_response_bytes: 1024 * MIB,
          morph_deadline_ms: 30_000,
          morph_max_nodes: 5_000_000,
        }),
      ),
    );
    expect(large.limits.maxResponseBytes).toBe(1024 * MIB);
    expect(large.limits.maxHtmlBytes).toBe(1024 * MIB);
    expect(large.limits.morphMaxNodes).toBe(5_000_000);
    expect(large.limits.morphDeadlineMs).toBe(30_000);
  });
});

describe("a tripped limit reaches the developer", () => {
  it("names the limit, the measured value, the configured value and the key", () => {
    const breach = limitBreach("maxResponseBytes", 20 * MIB, 16 * MIB, { atLeast: true });
    expect(breach.message).toBe(
      "Suprnova Live response size limit exceeded: measured at least 20971520 bytes, " +
        "configured 16777216 bytes. Raise LIVE_MAX_RESPONSE_BYTES in the application's " +
        ".env file to allow it.",
    );
  });

  it("writes the message to the browser console unless diagnostics are off", () => {
    const breach = limitBreach("morphMaxKeys", 12, 10);
    const console = { error: vi.fn() };
    new CoreRuntimeDiagnostics("errors", console).limit(breach);
    expect(console.error).toHaveBeenCalledWith(breach.message);
    const silent = { error: vi.fn() };
    new CoreRuntimeDiagnostics("off", silent).limit(breach);
    expect(silent.error).not.toHaveBeenCalled();
  });
});
