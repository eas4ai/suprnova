import { describe, expect, it } from "vitest";

import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import type { BuiltLiveRequest } from "../src/transport/request.js";
import { LiveTransportError, liveMediaType } from "../src/transport/fetch.js";
import { retryLiveRequest, type RetryPolicy } from "../src/transport/retry.js";

const POLICY: RetryPolicy = Object.freeze({
  baseDelayMs: 10,
  jitterRatio: 0,
  maximumAttempts: 4,
  maximumDelayMs: 100,
  retryableStatuses: Object.freeze([502, 503, 504]),
});

function request(): BuiltLiveRequest {
  return Object.freeze({
    identity: Object.freeze({
      baseRevision: 7n,
      correlationId: "EBESExQVFhcYGRobHB0eHw",
      idempotencyKey: "MDEyMzQ1Njc4OTo7PD0-Pw",
      promotionNonce: null,
      semanticDigest: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    }),
    mediaType: liveMediaType(1),
    protocolVersion: 1,
    text: "immutable-body",
  });
}

function time(): { clock: RuntimeClock; scheduler: RuntimeScheduler; delays: number[] } {
  let now = 1_000;
  let handle = 0;
  const delays: number[] = [];
  return {
    clock: { now: () => now },
    delays,
    scheduler: {
      animationFrame: () => 1,
      cancelAnimationFrame: () => undefined,
      clearTimeout: () => undefined,
      microtask: queueMicrotask,
      timeout(callback, milliseconds) {
        delays.push(milliseconds);
        handle += 1;
        now += milliseconds;
        queueMicrotask(callback);
        return handle;
      },
    },
  };
}

describe("safe Live retries", () => {
  it("reuses exact request bytes and identity with bounded exponential delays", async () => {
    const fixture = time();
    const seen: BuiltLiveRequest[] = [];
    let attempt = 0;
    const result = await retryLiveRequest(request(), {
      attempt: (candidate) => {
        seen.push(candidate);
        attempt += 1;
        if (attempt === 1) return Promise.reject(new LiveTransportError("network"));
        if (attempt === 2) return Promise.reject(new LiveTransportError("http", 503));
        return Promise.resolve(
          Object.freeze({ protocolVersion: 1 as const, status: 200, text: "accepted" }),
        );
      },
      clock: fixture.clock,
      isOnline: () => true,
      jitter: () => 0,
      policy: POLICY,
      scheduler: fixture.scheduler,
    });

    expect(result.attempts).toBe(3);
    expect(fixture.delays).toEqual([10, 20]);
    expect(seen).toHaveLength(3);
    expect(seen.every((candidate) => candidate === seen[0])).toBe(true);
  });

  it("never retries cancellation, incompatible responses, or unlisted statuses", async () => {
    for (const error of [
      new LiveTransportError("aborted"),
      new LiveTransportError("media"),
      new LiveTransportError("correlation"),
      new LiveTransportError("http", 409),
    ]) {
      const fixture = time();
      let attempts = 0;
      await expect(
        retryLiveRequest(request(), {
          attempt: () => {
            attempts += 1;
            return Promise.reject(error);
          },
          clock: fixture.clock,
          isOnline: () => true,
          jitter: () => 0,
          policy: POLICY,
          scheduler: fixture.scheduler,
        }),
      ).rejects.toBe(error);
      expect(attempts).toBe(1);
      expect(fixture.delays).toEqual([]);
    }
  });

  it("snapshots retry semantics before the first attempt", async () => {
    const fixture = time();
    const mutablePolicy = {
      baseDelayMs: 10,
      jitterRatio: 0,
      maximumAttempts: 2,
      maximumDelayMs: 100,
      retryableStatuses: [503],
    };
    let attempts = 0;
    const result = await retryLiveRequest(request(), {
      attempt: () => {
        attempts += 1;
        mutablePolicy.maximumAttempts = 1;
        return attempts === 1
          ? Promise.reject(new LiveTransportError("network"))
          : Promise.resolve(
              Object.freeze({ protocolVersion: 1 as const, status: 200, text: "accepted" }),
            );
      },
      clock: fixture.clock,
      isOnline: () => true,
      jitter: () => 0,
      policy: mutablePolicy,
      scheduler: fixture.scheduler,
    });
    expect(result.attempts).toBe(2);
  });

  it("stops when offline remains false, attempts exhaust, or cancellation arrives during delay", async () => {
    const offline = time();
    let offlineAttempts = 0;
    await expect(
      retryLiveRequest(request(), {
        attempt: () => {
          offlineAttempts += 1;
          return Promise.reject(new LiveTransportError("offline"));
        },
        clock: offline.clock,
        isOnline: () => false,
        jitter: () => 0,
        policy: POLICY,
        scheduler: offline.scheduler,
      }),
    ).rejects.toMatchObject({ kind: "offline" });
    expect(offlineAttempts).toBe(1);

    const exhausted = time();
    let exhaustedAttempts = 0;
    await expect(
      retryLiveRequest(request(), {
        attempt: () => {
          exhaustedAttempts += 1;
          return Promise.reject(new LiveTransportError("network"));
        },
        clock: exhausted.clock,
        isOnline: () => true,
        jitter: () => 0,
        policy: Object.freeze({ ...POLICY, maximumAttempts: 2 }),
        scheduler: exhausted.scheduler,
      }),
    ).rejects.toMatchObject({ kind: "network" });
    expect(exhaustedAttempts).toBe(2);

    const cancellation = new AbortController();
    const delayed = time();
    delayed.scheduler.timeout = (callback, milliseconds) => {
      delayed.delays.push(milliseconds);
      cancellation.abort();
      queueMicrotask(callback);
      return 1;
    };
    let canceledAttempts = 0;
    await expect(
      retryLiveRequest(request(), {
        attempt: () => {
          canceledAttempts += 1;
          return Promise.reject(new LiveTransportError("network"));
        },
        clock: delayed.clock,
        isOnline: () => true,
        jitter: () => 0,
        policy: POLICY,
        scheduler: delayed.scheduler,
        signal: cancellation.signal,
      }),
    ).rejects.toMatchObject({ kind: "aborted" });
    expect(canceledAttempts).toBe(1);
  });
});
