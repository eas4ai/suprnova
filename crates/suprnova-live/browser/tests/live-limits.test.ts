import { readFileSync } from "node:fs";

import { describe, expect, it, vi } from "vitest";

import { decodeAsyncEnvelope } from "../src/async-updates/envelope.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import { canonicalize, parseCanonicalJson, type JsonValue } from "../src/canonical.js";
import type { RuntimeFeatureDocumentContext } from "../src/features/contract.js";
import { breachOf, limitBreach, SERVER_DEFAULT_LIMITS, type LiveLimits } from "../src/limits.js";
import { IdiomorphAdapter } from "../src/morph/idiomorph.js";
import { morphLimitsFrom } from "../src/morph/limits.js";
import { preflightIslandMorph } from "../src/morph/preflight.js";
import { decodeSnapshotPublicView } from "../src/protocol/snapshot-view.js";
import { ProtocolValidationError, validateUpdateResponse } from "../src/protocol.js";
import { CONFIG_ELEMENT_ID, parseRuntimeConfig } from "../src/runtime/config.js";
import { CoreRuntimeDiagnostics } from "../src/runtime/diagnostics.js";
import { resolveUploadManagerOptions } from "../src/uploads/feature.js";
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

describe("redirect URLs the server sends", () => {
  function v1Redirect(target: string): string {
    const fixtures = JSON.parse(
      readFileSync(new URL("../../fixtures/v1/protocol-success.json", import.meta.url), "utf8"),
    ) as { cases: { id: string; encoded: string }[] };
    const found = fixtures.cases.find((candidate) => candidate.id === "redirect-response");
    if (found === undefined) throw new Error("missing fixture redirect-response");
    const response = JSON.parse(found.encoded) as Json;
    response["redirect"] = target;
    return canonicalize(response);
  }

  function v2Redirect(target: string): string {
    const response = protocolFixture("child-delivery-response");
    for (const key of ["accepted_revision", "render", "snapshot"]) {
      Reflect.deleteProperty(response, key);
    }
    response["child_deliveries"] = [];
    response["url_intent"] = null;
    response["redirect"] = target;
    return canonicalize(response);
  }

  const long = `/reports?${"q=x&".repeat(2_500)}`;

  it("follows a 10,000-byte redirect, past the old 2,048-byte bound", () => {
    expect(long.length).toBeGreaterThan(10_000);
    for (const encoded of [v1Redirect(long), v2Redirect(long)]) {
      expect(
        refusal(() => {
          validateUpdateResponse(encoded);
        }),
      ).toBeNull();
    }
  });

  it("refuses a redirect over the configured size and names the key", () => {
    for (const encoded of [v1Redirect(long), v2Redirect(long)]) {
      const error = refusal(() => {
        validateUpdateResponse(encoded, limits({ maxRedirectBytes: 4_096 }));
      });
      expect(error).toBeInstanceOf(ProtocolValidationError);
      expect((error as ProtocolValidationError).code).toBe("unsafe_redirect");
      expect(breachOf(error)?.key).toBe("LIVE_MAX_REDIRECT_BYTES");
      expect((error as Error).message).toContain(
        `redirect URL size limit exceeded (redirect): measured ${String(long.length)} bytes, ` +
          "configured 4096 bytes",
      );
    }
  });
});

describe("the boot configuration carries every limit", () => {
  const LIMITS_CONFIG = {
    async_max_queued_events: 4_096,
    async_max_replay_events: 4_096,
    max_html_bytes: 16 * MIB,
    max_json_depth: 32,
    max_json_entries: 1_000_000,
    max_redirect_bytes: 65_536,
    max_request_bytes: 16 * MIB,
    max_request_items: 65_536,
    max_response_items: 65_536,
    morph_deadline_ms: 0,
    morph_max_attributes: 10_000_000,
    morph_max_attributes_per_element: 4_096,
    morph_max_depth: 512,
    morph_max_keys: 1_000_000,
    morph_max_nodes: 1_000_000,
    upload_chunk_bytes: 8 * MIB,
    upload_max_active: 8,
    upload_max_file_bytes: 1024 * MIB,
    upload_max_pending_bytes: 4096 * MIB,
    upload_max_pending_files: 1_024,
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

  it("reads the upload, asynchronous queue and redirect limits the server configured", () => {
    const parsed = parseRuntimeConfig(
      configDocument(
        config({
          async_max_queued_events: 65_536,
          async_max_replay_events: 20_000,
          max_redirect_bytes: 2_097_152,
          max_response_bytes: 16 * MIB,
          upload_chunk_bytes: 64 * MIB,
          upload_max_active: 64,
          upload_max_file_bytes: 1024 * 1024 * MIB,
          upload_max_pending_bytes: 4096 * 1024 * MIB,
          upload_max_pending_files: 65_536,
        }),
      ),
    );
    expect(parsed.limits.asyncMaxQueuedEvents).toBe(65_536);
    expect(parsed.limits.asyncMaxReplayEvents).toBe(20_000);
    expect(parsed.limits.maxRedirectBytes).toBe(2_097_152);
    expect(parsed.limits.uploadChunkBytes).toBe(64 * MIB);
    expect(parsed.limits.uploadMaxActive).toBe(64);
    expect(parsed.limits.uploadMaxFileBytes).toBe(1024 * 1024 * MIB);
    expect(parsed.limits.uploadMaxPendingBytes).toBe(4096 * 1024 * MIB);
    expect(parsed.limits.uploadMaxPendingFiles).toBe(65_536);
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

describe("upload settings come from the server's configuration", () => {
  const transport = { send: () => Promise.reject(new Error("unused")) };
  const ports = {
    connectivity: { online: () => true },
    randomness: { idempotencyKey: () => "key" },
    transport,
  };
  function context(limits?: LiveLimits): RuntimeFeatureDocumentContext {
    return {
      diagnose: () => undefined,
      ...(limits === undefined ? {} : { limits }),
      onDispose: () => undefined,
    };
  }

  it("uses the configured chunk, transfer, file and pending limits", () => {
    const configured = resolveUploadManagerOptions(
      ports,
      context({
        ...SERVER_DEFAULT_LIMITS,
        uploadChunkBytes: MIB,
        uploadMaxActive: 3,
        uploadMaxFileBytes: 10 * MIB,
        uploadMaxPendingBytes: 20 * MIB,
        uploadMaxPendingFiles: 5,
      }),
    );
    expect(configured).toMatchObject({
      chunkBytes: MIB,
      maxActive: 3,
      maxFileBytes: 10 * MIB,
      maxItems: 5,
      maxQueueBytes: 20 * MIB,
    });
    expect(resolveUploadManagerOptions(ports, context())).toMatchObject({
      chunkBytes: 8 * MIB,
      maxActive: 8,
      maxFileBytes: 1024 * MIB,
      maxItems: 1_024,
      maxQueueBytes: 4096 * MIB,
    });
  });

  it("lets an application option lower a limit but never raise it", () => {
    const lowered = resolveUploadManagerOptions(
      { ...ports, chunkBytes: 64 * MIB, maxActive: 2, maxItems: 1, maxQueueBytes: MIB },
      context(SERVER_DEFAULT_LIMITS),
    );
    expect(lowered).toMatchObject({
      chunkBytes: 8 * MIB,
      maxActive: 1,
      maxItems: 1,
      maxQueueBytes: MIB,
    });
  });
});
