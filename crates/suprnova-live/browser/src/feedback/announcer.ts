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

const MAX_TRACKED_ANNOUNCEMENTS = 256;
const DEFAULT_MAXIMUM_PER_WINDOW = 8;
const DEFAULT_WINDOW_MS = 5_000;
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
  readonly #maximumPerWindow: number;
  readonly #now: () => number;
  readonly #sink: FeedbackAnnouncementSink;
  readonly #seenAt = new Map<string, number>();
  readonly #acceptedAt: number[] = [];
  readonly #windowMs: number;

  constructor(
    sink: FeedbackAnnouncementSink,
    options: Readonly<{
      maximumPerWindow?: number;
      now?: () => number;
      windowMs?: number;
    }> = {},
  ) {
    this.#sink = sink;
    this.#maximumPerWindow = options.maximumPerWindow ?? DEFAULT_MAXIMUM_PER_WINDOW;
    this.#now = options.now ?? Date.now;
    this.#windowMs = options.windowMs ?? DEFAULT_WINDOW_MS;
    if (
      !Number.isSafeInteger(this.#maximumPerWindow) ||
      this.#maximumPerWindow < 1 ||
      this.#maximumPerWindow > 32 ||
      !Number.isSafeInteger(this.#windowMs) ||
      this.#windowMs < 1 ||
      this.#windowMs > 60_000
    ) {
      throw new RangeError("feedback_announcement_policy_invalid");
    }
  }

  announce(
    scope: string,
    kind: FeedbackAnnouncementKind,
    transition: string,
    politeness: FeedbackPoliteness,
  ): boolean {
    if (!SAFE_KEY.test(scope) || !SAFE_KEY.test(transition)) return false;
    const now = this.#now();
    if (!Number.isSafeInteger(now) || now < 0) return false;
    const threshold = now - this.#windowMs;
    while ((this.#acceptedAt[0] ?? now) <= threshold) this.#acceptedAt.shift();
    for (const [tracked, accepted] of this.#seenAt) {
      if (accepted <= threshold) this.#seenAt.delete(tracked);
    }
    const key = `${scope}:${kind}:${transition}`;
    if (this.#seenAt.has(key) || this.#acceptedAt.length >= this.#maximumPerWindow) return false;
    try {
      this.#sink(Object.freeze({ message: MESSAGES[kind], politeness }));
    } catch {
      return false;
    }
    this.#seenAt.set(key, now);
    this.#acceptedAt.push(now);
    if (this.#seenAt.size > MAX_TRACKED_ANNOUNCEMENTS) {
      const expired = this.#seenAt.keys().next().value;
      if (typeof expired === "string") this.#seenAt.delete(expired);
    }
    return true;
  }
}
