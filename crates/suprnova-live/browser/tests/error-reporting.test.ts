import { describe, expect, it, vi } from "vitest";
import { REPORTING_SECRET_CASES } from "./support/reporting-secrets.js";
import { BrowserErrorReporter } from "../src/runtime/reporting.js";

function host() {
  const target = new EventTarget();
  const fetch = vi
    .fn<(url: string, options: RequestInit) => Promise<{ ok: boolean }>>()
    .mockResolvedValue({ ok: true });
  return {
    target,
    fetch,
    window: Object.assign(target, {
      fetch,
      crypto: globalThis.crypto,
      setTimeout,
      clearTimeout,
    }) as unknown as Window,
  };
}

async function drain() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function errorEvent(type: string, fields: Record<string, unknown>): Event {
  return Object.assign(new Event(type), fields);
}

describe("REPORT-007 Live browser reporting", () => {
  it("sends uncaught errors, rejections and hydration diagnostics as envelopes", async () => {
    const h = host();
    const reporter = new BrowserErrorReporter(h.window, "https://public@tracker.test/prefix/42");
    h.target.dispatchEvent(errorEvent("error", { error: new TypeError("uncaught failure") }));
    h.target.dispatchEvent(
      errorEvent("unhandledrejection", { reason: new Error("rejected failure") }),
    );
    reporter.record({
      code: "island_invalid",
      detailCode: "invalid_shape",
      phase: "discovery",
      severity: "error",
    });
    await drain();
    expect(h.fetch).toHaveBeenCalledTimes(3);
    const [endpoint, options] = h.fetch.mock.calls[0] as [string, RequestInit];
    expect(endpoint).toBe(
      "https://tracker.test/prefix/api/42/envelope/?sentry_key=public&sentry_version=7",
    );
    expect(options.credentials).toBe("omit");
    const lines = (options.body as string).split("\n");
    expect(JSON.parse(lines[1] ?? "null")).toEqual({ type: "event" });
    expect(JSON.parse(lines[2] ?? "null") as unknown).toMatchObject({
      exception: { values: [{ type: "TypeError", value: "uncaught failure" }] },
    });
    reporter.dispose();
    h.target.dispatchEvent(errorEvent("error", { error: new Error("after stop") }));
    await drain();
    expect(h.fetch).toHaveBeenCalledTimes(3);
  });

  it("reports on HTTP origins where crypto.randomUUID is unavailable", async () => {
    const h = host();
    Reflect.set(h.window, "crypto", {
      getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto),
    });
    const reporter = new BrowserErrorReporter(h.window, "http://public@tracker.test/42");
    reporter.capture(new Error("HTTP application error"));
    await drain();
    expect(h.fetch).toHaveBeenCalledTimes(1);
    reporter.dispose();
  });

  it("bounds bursts, deduplicates and isolates failed network delivery", async () => {
    const h = host();
    h.fetch.mockRejectedValue(new Error("offline"));
    const reporter = new BrowserErrorReporter(h.window, "https://public@tracker.test/42");
    for (let index = 0; index < 100; index += 1) {
      reporter.capture(new Error(`failure ${String(index)}`));
    }
    reporter.capture(new Error("failure 0"));
    await drain();
    expect(h.fetch.mock.calls.length).toBeLessThanOrEqual(16);
    reporter.dispose();
  });

  it("redacts complete credential headers, quoted secrets and encoded parameter names", async () => {
    for (const { text, secrets } of REPORTING_SECRET_CASES) {
      const h = host();
      const reporter = new BrowserErrorReporter(h.window, "https://public@tracker.test/42");
      reporter.capture(new Error(text));
      await drain();
      expect(h.fetch).toHaveBeenCalledTimes(1);
      const body = h.fetch.mock.calls[0]?.[1].body as string;
      for (const secret of secrets) expect(body).not.toContain(secret);
      reporter.dispose();
    }
  });

  it("rejects malformed or secret DSNs and excludes page state and URL secrets", async () => {
    const h = host();
    for (const dsn of [
      "",
      "javascript:alert(1)",
      "https://public:secret@tracker.test/42",
      "https://tracker.test/42",
      "https://public@tracker.test/42?secret=x",
    ]) {
      expect(() => new BrowserErrorReporter(h.window, dsn)).toThrow();
    }
    const reporter = new BrowserErrorReporter(h.window, "https://public@tracker.test/42");
    reporter.capture(
      new Error(
        "password=hunter2 Authorization: Bearer topsecret https://app.test/path?token=secret",
      ),
    );
    await drain();
    const body = h.fetch.mock.calls[0]?.[1].body as string;
    expect(body).not.toContain("hunter2");
    expect(body).not.toContain("topsecret");
    expect(body).not.toContain("token=secret");
    expect(body).not.toContain('"request"');
    reporter.dispose();
  });
});
