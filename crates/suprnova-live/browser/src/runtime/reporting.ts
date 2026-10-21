import { ENGINE_VERSION } from "../version.js";
import type { RuntimeDiagnosticInput } from "./diagnostics.js";

// Public DSNs carry an ingestion key, never the server's private credentials.
export function reportingEndpoint(dsn: string): URL {
  const url = new URL(dsn);
  const segments = url.pathname.split("/").filter(Boolean);
  const project = segments.pop();
  if (
    Array.from(dsn).some(
      (character) =>
        character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127 || /\s/u.test(character),
    ) ||
    !["http:", "https:"].includes(url.protocol) ||
    !url.username ||
    url.password ||
    url.search ||
    url.hash ||
    project === undefined ||
    !/^[0-9]+$/u.test(project) ||
    !/^[A-Za-z0-9_-]+$/u.test(url.username) ||
    dsn.includes("\\")
  )
    throw new TypeError("invalid_public_reporting_dsn");
  const key = url.username;
  url.username = "";
  url.pathname = `${segments.length ? `/${segments.join("/")}` : ""}/api/${project}/envelope/`;
  url.search = new URLSearchParams({ sentry_key: key, sentry_version: "7" }).toString();
  return url;
}

function scrub(text: string): string {
  const bounded = text.slice(0, 16_384);
  const urls = bounded.replace(/https?:\/\/[^\s)]+/gu, (value) => {
    try {
      const url = new URL(value);
      return `${url.origin}${url.pathname}`;
    } catch {
      return "[URL]";
    }
  });
  const fields = /(^|[\s?&#;,{"'])([^\s?&#;=,:{}"'<>`]+)(["']?\s*[:=]\s*)/gu;
  let redacted = "";
  let cursor = 0;
  let match: RegExpExecArray | null;
  while ((match = fields.exec(urls)) !== null) {
    const name = match[2] ?? "";
    // Decode valid escapes even beside an invalid percent sign.
    const decoded = new URLSearchParams(`${name}=`).keys().next().value ?? name;
    if (
      !/(?:password|token|secret|key|signature|authorization|cookie|[cx]srf|session)/iu.test(
        decoded,
      )
    )
      continue;
    // Error text has no trustworthy value boundary. Discard the rest of a
    // credential line, including every cookie and quoted or malformed value.
    const lineEnd = urls.slice(fields.lastIndex).search(/[\r\n]/u);
    redacted += `${urls.slice(cursor, fields.lastIndex)}[REDACTED]`;
    cursor = lineEnd === -1 ? urls.length : fields.lastIndex + lineEnd;
    fields.lastIndex = cursor;
  }
  return (redacted + urls.slice(cursor)).slice(0, 2048);
}

function exception(error: unknown): Record<string, unknown> {
  const known = error instanceof Error;
  const frames = known
    ? (error.stack ?? "")
        .split("\n")
        .slice(1, 17)
        .flatMap((line) => {
          const match = /(?:at (.*?) \()?((?:https?:\/\/)[^\s)]+):(\d+):(\d+)\)?$/u.exec(
            line.trim(),
          );
          if (match === null) return [];
          return [
            {
              filename: scrub(match[2] ?? "").slice(0, 512),
              function: scrub(match[1] ?? "?").slice(0, 128),
              lineno: Number(match[3]),
              colno: Number(match[4]),
            },
          ];
        })
        .reverse()
    : [];
  return {
    type: known ? scrub(error.name) : "UnhandledRejection",
    value: known
      ? scrub(error.message)
      : typeof error === "string"
        ? scrub(error)
        : "Non-Error rejection",
    ...(frames.length ? { stacktrace: { frames } } : {}),
  };
}

/// Bounded, dependency-free ingestion keeps Live usable when a tracker is down.
export class BrowserErrorReporter {
  readonly #window: Window;
  readonly #endpoint: string;
  readonly #seen = new Map<string, number>();
  #pending = 0;
  #disposed = false;
  readonly #error = (event: Event): void => {
    const error = event as ErrorEvent;
    this.capture(error.error ?? error.message ?? "Uncaught browser error");
  };
  readonly #rejection = (event: Event): void => {
    this.capture((event as PromiseRejectionEvent).reason);
  };

  constructor(window: Window, dsn: string) {
    this.#endpoint = reportingEndpoint(dsn).href;
    this.#window = window;
    window.addEventListener("error", this.#error);
    window.addEventListener("unhandledrejection", this.#rejection);
  }

  record(input: RuntimeDiagnosticInput): void {
    if (input.severity === "error") {
      this.capture(new Error(`Live ${input.phase}: ${input.code} (${input.detailCode})`));
    }
  }

  capture(error: unknown): void {
    if (this.#disposed || this.#pending >= 16) return;
    try {
      const value = exception(error);
      const fingerprint = JSON.stringify(value);
      const now = Date.now();
      if (now - (this.#seen.get(fingerprint) ?? 0) < 60_000) return;
      if (this.#seen.size >= 128) this.#seen.delete(this.#seen.keys().next().value ?? "");
      this.#seen.set(fingerprint, now);
      const eventId = Array.from(this.#window.crypto.getRandomValues(new Uint8Array(16)))
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join("");
      const body = [
        JSON.stringify({ event_id: eventId, sent_at: new Date().toISOString() }),
        JSON.stringify({ type: "event" }),
        JSON.stringify({
          event_id: eventId,
          timestamp: now / 1000,
          platform: "javascript",
          level: "error",
          sdk: { name: "suprnova.live.browser", version: ENGINE_VERSION },
          exception: { values: [value] },
        }),
      ].join("\n");
      this.#pending += 1;
      void this.#send(body);
    } catch {
      // Hostile rejection values and unavailable browser APIs cannot break Live.
    }
  }

  dispose(): void {
    this.#disposed = true;
    this.#window.removeEventListener("error", this.#error);
    this.#window.removeEventListener("unhandledrejection", this.#rejection);
    this.#seen.clear();
  }

  async #send(body: string): Promise<void> {
    const controller = new AbortController();
    const timeout = this.#window.setTimeout(() => {
      controller.abort();
    }, 2000);
    try {
      await this.#window.fetch(this.#endpoint, {
        method: "POST",
        body,
        headers: { "Content-Type": "text/plain;charset=UTF-8" },
        credentials: "omit",
        referrerPolicy: "no-referrer",
        keepalive: true,
        signal: controller.signal,
      });
    } catch {
      // Reporting is best effort, with no retry or recursive error event.
    } finally {
      this.#window.clearTimeout(timeout);
      this.#pending -= 1;
    }
  }
}
