import type { JsonValue } from "../canonical.js";

export type SubscriptionState =
  "disconnected" | "connecting" | "current" | "degraded" | "reconnecting" | "closed";

export interface StreamPosition {
  readonly epoch: bigint;
  readonly sequence: bigint;
}

export type AsyncPayloadSchema = "json" | "null" | "boolean" | "i64" | "u64" | "f64" | "string";

export type AsyncEventCycle =
  | Readonly<{ kind: "forbid_repeated_island" }>
  | Readonly<{ kind: "maximum_hops"; maximumHops: number }>;

export interface AsyncRegisteredEventContract {
  readonly cycle: AsyncEventCycle;
  readonly maximumFanout: number;
  readonly name: string;
  readonly order: "per_source_sequence";
  readonly payloadContract: string;
  readonly schema: AsyncPayloadSchema;
  readonly source: "stream";
  readonly targets: readonly string[];
  readonly version: number;
}

export interface AsyncPresentationSignalContract {
  readonly name: string;
  readonly schema: AsyncPayloadSchema;
  readonly scope: string;
}

export type AsyncPayload =
  | Readonly<{ kind: "refresh"; name: "refresh" }>
  | Readonly<{
      event: string;
      kind: "browser_event";
      payload: JsonValue;
      schema_version: number;
      target: string;
    }>
  | Readonly<{ kind: "presentation_signal"; name: string; scope: string; value: JsonValue }>
  | Readonly<{ kind: "heartbeat" }>
  | Readonly<{
      kind: "complete";
      reason: "server_shutdown" | "subscription_retired" | "stream_completed";
    }>
  | Readonly<{
      code: "authorization_lost" | "replay_unavailable" | "backpressure" | "stream_unavailable";
      kind: "error";
    }>;

export interface AsyncEnvelope {
  readonly payload: AsyncPayload;
  readonly position: StreamPosition;
  readonly protocolVersion: 1;
  readonly stream: string;
  readonly subscriptionId: string;
}

declare const VALIDATED_ASYNC_ENVELOPE: unique symbol;

/** A canonical envelope that passed the closed membership-aware decoder. */
export interface ValidatedAsyncEnvelope extends AsyncEnvelope {
  readonly [VALIDATED_ASYNC_ENVELOPE]: never;
}

export type AsyncTransportKind = "sse" | "websocket";

export interface DocumentTransportKey {
  readonly authorizationScope: string;
  readonly origin: string;
  readonly transport: AsyncTransportKind;
}

export type AsyncTransportAuthorization =
  Readonly<{ kind: "session_cookie" }> | Readonly<{ credential: string; kind: "bearer" }>;

export type AsyncReconnectPolicy =
  | Readonly<{
      kind: "refresh_on_reconnect";
      maximumAttempts: number;
      maximumDelayMs: number;
      minimumDelayMs: number;
    }>
  | Readonly<{
      kind: "resume_or_refresh";
      maximumAttempts: number;
      maximumDelayMs: number;
      minimumDelayMs: number;
    }>;

export interface AuthorizedLogicalSubscription {
  readonly authorization: AsyncTransportAuthorization;
  readonly baseline: StreamPosition;
  readonly descriptorBinding: string;
  readonly document: DocumentTransportKey;
  readonly events: readonly AsyncRegisteredEventContract[];
  readonly expiresAt: number;
  readonly fallbackPoll: PollFallbackPolicy;
  readonly heartbeatTimeoutMs: number;
  readonly presentationSignals: readonly AsyncPresentationSignalContract[];
  readonly reconnect: AsyncReconnectPolicy;
  readonly stream: string;
  readonly subscriptionId: string;
}

export interface PollFallbackPolicy {
  readonly intervalMs: number;
  readonly jitterRatio: number;
  readonly initial: "wait" | "immediate";
  readonly visibility: "visible" | "always";
}

export type AsyncReceiveDisposition =
  | "applied"
  | "pending"
  | "duplicate"
  | "stale"
  | "gap"
  | "continuity_required"
  | "dispatch_failed"
  | "closed";

export interface AsyncClock {
  now(): number;
}

export interface AsyncRandomness {
  number(): number;
}

export interface AsyncTimerPort {
  clearTimeout(handle: number): void;
  timeout(callback: VoidFunction, milliseconds: number): number;
}
