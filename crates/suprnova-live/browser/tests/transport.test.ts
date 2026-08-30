import { describe, expect, it } from "vitest";

import type { RuntimeScheduler, TransportPort } from "../src/runtime/ports.js";
import type { BuiltLiveRequest } from "../src/transport/request.js";
import {
  fetchLiveRequest,
  LiveTransportError,
  liveMediaType,
  transportFailureDiagnostic,
} from "../src/transport/fetch.js";

const CORRELATION = "EBESExQVFhcYGRobHB0eHw";

function request(version: 1 | 2 = 1): BuiltLiveRequest {
  return Object.freeze({
    identity: Object.freeze({
      baseRevision: 7n,
      correlationId: CORRELATION,
      idempotencyKey: "MDEyMzQ1Njc4OTo7PD0-Pw",
      promotionNonce: null,
      semanticDigest: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    }),
    mediaType: liveMediaType(version),
    protocolVersion: version,
    text: "{}",
  });
}

function responseText(correlation = CORRELATION): string {
  return JSON.stringify({
    correlation_id: correlation,
    effects: [],
    events: [],
    extensions: {},
    outcome: "accepted",
    protocol_version: 1,
    redirect: "/profiles",
    validation: {},
  });
}

function responseTextV2(): string {
  return JSON.stringify({
    child_deliveries: [],
    correlation_id: CORRELATION,
    effects: [],
    events: [],
    extensions: {},
    outcome: "accepted",
    protocol_version: 2,
    redirect: "/profiles",
    url_intent: null,
    validation: {},
  });
}

function scheduler(onTimeout?: (callback: VoidFunction) => void): RuntimeScheduler {
  return {
    animationFrame: () => 1,
    cancelAnimationFrame: () => undefined,
    clearTimeout: () => undefined,
    microtask: queueMicrotask,
    timeout(callback) {
      onTimeout?.(callback);
      return 1;
    },
  };
}

function options(transport: TransportPort, overrides: Record<string, unknown> = {}) {
  return {
    credentials: "same-origin" as const,
    endpoint: new URL("https://example.test/live"),
    isOnline: () => true,
    maxResponseBytes: 4_096,
    requestTimeoutMs: 5_000,
    scheduler: scheduler(),
    transport,
    ...overrides,
  };
}

async function failure(promise: Promise<unknown>): Promise<LiveTransportError> {
  try {
    await promise;
    throw new Error("transport unexpectedly succeeded");
  } catch (error: unknown) {
    expect(error).toBeInstanceOf(LiveTransportError);
    if (!(error instanceof LiveTransportError)) throw error;
    return error;
  }
}

describe("Live fetch transport", () => {
  it("treats user cancellation as an ordinary terminal outcome", () => {
    expect(transportFailureDiagnostic(new LiveTransportError("aborted"))).toBeNull();
    expect(transportFailureDiagnostic(new LiveTransportError("correlation"))).toMatchObject({
      code: "transport_failed",
      detailCode: "invalid_response",
      phase: "transport",
      severity: "error",
    });
  });

  it("posts immutable bytes only to the configured endpoint with exact transport policy", async () => {
    let captured: { input: RequestInfo | URL; init?: RequestInit } | null = null;
    const transport: TransportPort = {
      fetch(input, init) {
        captured = { input, ...(init === undefined ? {} : { init }) };
        return Promise.resolve(
          new Response(responseText(), {
            headers: { "content-type": liveMediaType(1) },
            status: 200,
          }),
        );
      },
    };

    const result = await fetchLiveRequest(request(), options(transport));
    expect(result.text).toBe(responseText());
    expect(captured).not.toBeNull();
    const admission = captured as unknown as { input: URL; init: RequestInit };
    expect(admission.input.href).toBe("https://example.test/live");
    expect(admission.init).toMatchObject({
      body: "{}",
      cache: "no-store",
      credentials: "same-origin",
      method: "POST",
    });
    const headers = new Headers(admission.init.headers);
    expect(headers.get("accept")).toBe(liveMediaType(1));
    expect(headers.get("content-type")).toBe(liveMediaType(1));
  });

  it("classifies status, media, size, and correlation failures without response detail", async () => {
    const cases = [
      {
        expected: "http",
        response: new Response(null, { status: 404 }),
      },
      {
        expected: "http",
        response: new Response(responseText(), {
          headers: { "content-type": liveMediaType(1) },
          status: 418,
        }),
      },
      {
        expected: "media",
        response: new Response(responseText(), {
          headers: { "content-type": "application/json" },
          status: 200,
        }),
      },
      {
        expected: "size",
        response: new Response(responseText(), {
          headers: { "content-length": "99999", "content-type": liveMediaType(1) },
          status: 200,
        }),
      },
      {
        expected: "correlation",
        response: new Response(responseText("MDEyMzQ1Njc4OTo7PD0-Pw"), {
          headers: { "content-type": liveMediaType(1) },
          status: 200,
        }),
      },
      {
        expected: "size",
        response: new Response("x".repeat(5_000), {
          headers: { "content-type": liveMediaType(1) },
          status: 200,
        }),
      },
      {
        expected: "protocol",
        response: new Response(responseTextV2(), {
          headers: { "content-type": liveMediaType(1) },
          status: 200,
        }),
      },
    ];
    for (const fixture of cases) {
      const error = await failure(
        fetchLiveRequest(
          request(),
          options({ fetch: () => Promise.resolve(fixture.response.clone()) }),
        ),
      );
      expect(error.kind).toBe(fixture.expected);
      expect(error.message).toBe(`live_transport_${fixture.expected}`);
    }
  });

  it("rejects an unsafe endpoint before invoking the transport port", async () => {
    let calls = 0;
    const error = await failure(
      fetchLiveRequest(
        request(),
        options(
          {
            fetch() {
              calls += 1;
              return Promise.reject(new Error("must not run"));
            },
          },
          { endpoint: new URL("ftp://example.test/live") },
        ),
      ),
    );
    expect(error.kind).toBe("unsafe_endpoint");
    expect(calls).toBe(0);
  });

  it("distinguishes timeout, user abort, offline, and online network failure", async () => {
    let fireTimeout: VoidFunction = () => undefined;
    const pending: TransportPort = {
      fetch: (_input, init) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    };
    const timed = fetchLiveRequest(
      request(),
      options(pending, {
        scheduler: scheduler((callback) => {
          fireTimeout = callback;
        }),
      }),
    );
    fireTimeout();
    expect((await failure(timed)).kind).toBe("timeout");

    const cancellation = new AbortController();
    const aborted = fetchLiveRequest(request(), options(pending, { signal: cancellation.signal }));
    cancellation.abort();
    expect((await failure(aborted)).kind).toBe("aborted");

    const broken = { fetch: () => Promise.reject(new TypeError("raw network detail")) };
    expect(
      (await failure(fetchLiveRequest(request(), options(broken, { isOnline: () => false })))).kind,
    ).toBe("offline");
    expect((await failure(fetchLiveRequest(request(), options(broken)))).kind).toBe("network");
  });
});
