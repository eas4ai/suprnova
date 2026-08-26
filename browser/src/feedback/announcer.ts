export type FeedbackAnnouncementKind =
  | "idle"
  | "dirty"
  | "queued"
  | "loading"
  | "validating"
  | "validation"
  | "interruption"
  | "offline"
  | "retry"
  | "failure"
  | "success"
  | "stream_disconnected"
  | "stream_connecting"
  | "stream_current"
  | "stream_degraded"
  | "stream_reconnecting"
  | "stream_closed";

export type FeedbackPoliteness = "polite" | "assertive";

export interface FeedbackAnnouncement {
  readonly message: string;
  readonly politeness: FeedbackPoliteness;
}

export type FeedbackAnnouncementSink = (announcement: FeedbackAnnouncement) => void;

const MAX_ANNOUNCEMENTS = 256;
const SAFE_KEY = /^[A-Za-z0-9_.:-]{1,192}$/u;
const MESSAGES: Readonly<Record<FeedbackAnnouncementKind, string>> = Object.freeze({
  dirty: "Unsaved changes",
  failure: "Request failed",
  idle: "Ready",
  interruption: "Request interrupted",
  loading: "Loading",
  offline: "Offline",
  queued: "Queued",
  retry: "Retrying",
  success: "Completed",
  stream_closed: "Updates closed",
  stream_connecting: "Connecting to updates",
  stream_current: "Updates current",
  stream_degraded: "Updates degraded",
  stream_disconnected: "Updates disconnected",
  stream_reconnecting: "Reconnecting to updates",
  validation: "Validation failed",
  validating: "Validating",
});

export class FeedbackAnnouncer {
  readonly #sink: FeedbackAnnouncementSink;
  readonly #seen = new Set<string>();
  readonly #order: string[] = [];

  constructor(sink: FeedbackAnnouncementSink) {
    this.#sink = sink;
  }

  announce(
    scope: string,
    kind: FeedbackAnnouncementKind,
    transition: string,
    politeness: FeedbackPoliteness,
  ): boolean {
    if (!SAFE_KEY.test(scope) || !SAFE_KEY.test(transition)) return false;
    const key = `${scope}:${kind}:${transition}`;
    if (this.#seen.has(key)) return false;
    this.#seen.add(key);
    this.#order.push(key);
    if (this.#order.length > MAX_ANNOUNCEMENTS) {
      const expired = this.#order.shift();
      if (expired !== undefined) this.#seen.delete(expired);
    }
    try {
      this.#sink(Object.freeze({ message: MESSAGES[kind], politeness }));
    } catch {
      return false;
    }
    return true;
  }
}
