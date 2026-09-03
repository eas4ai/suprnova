/**
 * Default browser host for the asynchronous feature.
 *
 * The feature stays inert until an application supplies clocks, timers,
 * randomness, transports, and an authority. A framework that serves the
 * reserved `/__live/v1/async/*` routes can hand the runtime this default host,
 * which issues and renews subscriptions and drives SSE membership control
 * against those routes with the browser's own credentials, and opens the
 * physical transports through the native `EventSource`, `WebSocket`, and
 * `fetch` APIs. Every server answer is decoded through a closed, fail-closed
 * mapping; nothing here interprets envelopes or event payloads.
 */

import { BrowserAsyncTransportPorts } from "./connections.js";
import type { SseMembershipControlRequest, SseMembershipOutcome } from "./connections.js";
import type {
  AsyncAuthorityPort,
  AsyncAuthorizationRequest,
  AsyncAuthorizationResult,
  AsyncFeatureOptions,
} from "./feature.js";
import type {
  AsyncReconnectPolicy,
  AsyncRegisteredEventContract,
  AsyncTimerPort,
  AsyncTransportAuthorization,
  AsyncTransportKind,
  AuthorizedLogicalSubscription,
  PollFallbackPolicy,
  StreamPosition,
} from "./types.js";

const SUBSCRIPTION_PATH = "/__live/v1/async/subscriptions";
const MEMBERSHIP_PATH = "/__live/v1/async/memberships";
const CONTROL_MARKER = "async-v1";
const MAX_CONTROL_RESPONSE_BYTES = 256 * 1024;
const MAX_REPLAY_ENVELOPES = 4096;
const MEMBERSHIP_TIMEOUT_MS = 10_000;
const MAX_TEXT_BYTES = 1024;

/** Options for the default browser host. */
export interface BrowserAsyncHostOptions {
  /** Fetch implementation; defaults to the global `fetch`. */
  readonly fetch?: typeof globalThis.fetch;
  /** Origin the reserved routes live on; defaults to the document origin. */
  readonly origin?: string;
  /** Transport the host asks the server for; the server decides the final kind. */
  readonly transport?: AsyncTransportKind;
}

interface ResolvedHost {
  readonly fetch: typeof globalThis.fetch;
  readonly origin: string;
  readonly transport: AsyncTransportKind;
}

function resolveHost(options: BrowserAsyncHostOptions | undefined): ResolvedHost {
  const fetchImpl = options?.fetch ?? globalThis.fetch.bind(globalThis);
  const origin = options?.origin ?? globalThis.location.origin;
  const transport = options?.transport ?? "sse";
  if (typeof fetchImpl !== "function") throw new Error("async_host_fetch_invalid");
  if (typeof origin !== "string" || new URL(origin).origin !== origin) {
    throw new Error("async_host_origin_invalid");
  }
  return Object.freeze({ fetch: fetchImpl, origin, transport });
}

function documentInstance(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}

async function boundedJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text.length > MAX_CONTROL_RESPONSE_BYTES) throw new Error("async_host_response_too_large");
  return JSON.parse(text) as unknown;
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("async_authority_invalid");
  }
  return value as Record<string, unknown>;
}

function text(value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_TEXT_BYTES) {
    throw new Error("async_authority_invalid");
  }
  return value;
}

function integer(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error("async_authority_invalid");
  }
  return value;
}

function decimal(value: unknown): bigint {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/u.test(value)) {
    throw new Error("async_authority_invalid");
  }
  return BigInt(value);
}

function position(value: unknown): StreamPosition {
  const fields = record(value);
  return Object.freeze({ epoch: decimal(fields["epoch"]), sequence: decimal(fields["sequence"]) });
}

function authorization(value: unknown): AsyncTransportAuthorization {
  const fields = record(value);
  const kind = fields["kind"];
  if (kind === "session_cookie") return Object.freeze({ kind: "session_cookie" as const });
  if (kind === "bearer") {
    return Object.freeze({ credential: text(fields["credential"]), kind: "bearer" as const });
  }
  throw new Error("async_authority_invalid");
}

function transportKind(value: unknown): AsyncTransportKind {
  if (value === "sse" || value === "websocket") return value;
  throw new Error("async_authority_invalid");
}

function fallbackPoll(value: unknown): PollFallbackPolicy {
  const fields = record(value);
  const initial = fields["initial"];
  const visibility = fields["visibility"];
  const jitter = fields["jitter_ratio"];
  if (
    (initial !== "wait" && initial !== "immediate") ||
    (visibility !== "visible" && visibility !== "always") ||
    typeof jitter !== "number" ||
    !Number.isFinite(jitter) ||
    jitter < 0 ||
    jitter > 1
  ) {
    throw new Error("async_authority_invalid");
  }
  return Object.freeze({
    initial,
    intervalMs: integer(fields["interval_ms"]),
    jitterRatio: jitter,
    visibility,
  });
}

function reconnect(value: unknown): AsyncReconnectPolicy {
  const fields = record(value);
  const kind = fields["kind"];
  if (kind !== "refresh_on_reconnect" && kind !== "resume_or_refresh") {
    throw new Error("async_authority_invalid");
  }
  return Object.freeze({
    kind,
    maximumAttempts: integer(fields["maximum_attempts"]),
    maximumDelayMs: integer(fields["maximum_delay_ms"]),
    minimumDelayMs: integer(fields["minimum_delay_ms"]),
  });
}

function eventContract(value: unknown): AsyncRegisteredEventContract {
  const fields = record(value);
  const cycle = record(fields["cycle"]);
  const cycleKind = cycle["kind"];
  const schema = fields["schema"];
  if (
    (cycleKind !== "forbid_repeated_island" && cycleKind !== "maximum_hops") ||
    fields["order"] !== "per_source_sequence" ||
    fields["source"] !== "stream" ||
    !Array.isArray(fields["targets"]) ||
    (schema !== "json" &&
      schema !== "null" &&
      schema !== "boolean" &&
      schema !== "i64" &&
      schema !== "u64" &&
      schema !== "f64" &&
      schema !== "string")
  ) {
    throw new Error("async_authority_invalid");
  }
  const targets = Object.freeze(fields["targets"].map(text));
  return Object.freeze({
    cycle:
      cycleKind === "forbid_repeated_island"
        ? Object.freeze({ kind: "forbid_repeated_island" as const })
        : Object.freeze({
            kind: "maximum_hops" as const,
            maximumHops: integer(cycle["maximumHops"]),
          }),
    maximumFanout: integer(fields["maximumFanout"] ?? fields["maximum_fanout"] ?? 1),
    name: text(fields["name"]),
    order: "per_source_sequence" as const,
    payloadContract:
      typeof fields["payloadContract"] === "string" ? fields["payloadContract"] : schema,
    schema,
    source: "stream" as const,
    targets,
    version: integer(fields["version"]),
  });
}

function presentationSignal(value: unknown) {
  const fields = record(value);
  const schema = fields["schema"];
  if (
    schema !== "null" &&
    schema !== "boolean" &&
    schema !== "i64" &&
    schema !== "u64" &&
    schema !== "string"
  ) {
    throw new Error("async_authority_invalid");
  }
  return Object.freeze({ name: text(fields["name"]), schema, scope: text(fields["scope"]) });
}

/** Decodes one issued or renewed subscription from the reserved control route. */
export function decodeAuthorizedSubscription(value: unknown): AuthorizedLogicalSubscription {
  const fields = record(value);
  const document = record(fields["document"]);
  const events = fields["events"];
  const signals = fields["presentation_signals"] ?? [];
  if (!Array.isArray(events) || !Array.isArray(signals)) throw new Error("async_authority_invalid");
  return Object.freeze({
    authorization: authorization(fields["authorization"]),
    baseline: position(fields["baseline"]),
    descriptorBinding: text(fields["descriptor_binding"]),
    document: Object.freeze({
      authorizationScope: text(document["authorization_scope"]),
      origin: text(document["origin"]),
      transport: transportKind(document["transport"]),
    }),
    events: Object.freeze(events.map(eventContract)),
    expiresAt: integer(fields["expires_at"]),
    fallbackPoll: fallbackPoll(fields["fallback_poll"]),
    heartbeatTimeoutMs: integer(fields["heartbeat_timeout_ms"]),
    presentationSignals: Object.freeze(signals.map(presentationSignal)),
    reconnect: reconnect(fields["reconnect"]),
    stream: text(fields["stream"]),
    subscriptionId: text(fields["subscription_id"]),
  });
}

function decodeAuthorization(value: unknown): AsyncAuthorizationResult {
  const fields = record(value);
  const replay = fields["replay"] ?? [];
  if (!Array.isArray(replay) || replay.length > MAX_REPLAY_ENVELOPES) {
    throw new Error("async_authority_invalid");
  }
  return Object.freeze({
    replay: Object.freeze(
      replay.map((entry) => {
        if (typeof entry !== "string") throw new Error("async_authority_invalid");
        return entry;
      }),
    ),
    subscription: decodeAuthorizedSubscription(fields["subscription"]),
  });
}

/**
 * Issues and renews logical subscriptions through the reserved subscription
 * route, carrying the browser's same-origin credentials and the control
 * marker the framework requires.
 */
export class BrowserAsyncAuthority implements AsyncAuthorityPort {
  readonly #host: ResolvedHost;
  readonly #documentInstance: string;

  constructor(options?: BrowserAsyncHostOptions) {
    this.#host = resolveHost(options);
    this.#documentInstance = documentInstance();
  }

  async authorize(request: AsyncAuthorizationRequest): Promise<AsyncAuthorizationResult> {
    const prior = request.prior;
    const body: Record<string, unknown> = {
      document_instance: this.#documentInstance,
      island: {
        component: request.identity.component,
        document_key: request.identity.documentKey,
        slot: request.identity.slot,
      },
      operation: prior === null ? "issue" : "renew",
      protocol_version: 1,
      stream: request.stream,
      transport: prior === null ? this.#host.transport : prior.document.transport,
    };
    if (prior !== null) {
      body["prior"] = {
        descriptor_binding: prior.descriptorBinding,
        subscription_id: prior.subscriptionId,
      };
    }
    if (request.position !== null) {
      body["position"] = {
        epoch: String(request.position.epoch),
        sequence: String(request.position.sequence),
      };
    }
    const response = await this.#host.fetch(new URL(SUBSCRIPTION_PATH, this.#host.origin).href, {
      body: JSON.stringify(body),
      cache: "no-store",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-Suprnova-Live": CONTROL_MARKER },
      method: "POST",
      redirect: "error",
      signal: request.signal,
    });
    if (response.status !== 200 && response.status !== 201) {
      throw new Error(`async_authority_rejected_${String(response.status)}`);
    }
    return decodeAuthorization(await boundedJson(response));
  }
}

function membershipRejectionReason(status: number): "authorization_lost" | "capacity" | "closed" {
  if (status === 401 || status === 403 || status === 404 || status === 410) {
    return "authorization_lost";
  }
  if (status === 409 || status === 429) return "capacity";
  return "closed";
}

/** Drives SSE membership control through the reserved membership route. */
export async function browserSseMembership(
  request: SseMembershipControlRequest,
  fetchImpl: typeof globalThis.fetch = globalThis.fetch.bind(globalThis),
): Promise<SseMembershipOutcome> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "X-Suprnova-Live": CONTROL_MARKER,
  };
  const authorization = request.subscription.authorization;
  if (authorization.kind === "bearer") {
    headers["Authorization"] = `SuprnovaAsync ${authorization.credential}`;
  }
  let response: Response;
  try {
    response = await fetchImpl(new URL(MEMBERSHIP_PATH, request.key.origin).href, {
      body: JSON.stringify({
        control_nonce: request.controlNonce,
        descriptor_binding: request.subscription.descriptorBinding,
        operation: request.operation,
        protocol_version: 1,
        stream: request.subscription.stream,
        subscription_id: request.subscription.subscriptionId,
        transport_generation: request.transportGeneration,
      }),
      cache: "no-store",
      credentials: "same-origin",
      headers,
      method: "POST",
      redirect: "error",
      signal: request.signal,
    });
  } catch {
    return Object.freeze({ kind: "rejected" as const, reason: "closed" as const });
  }
  if (response.status !== 200) {
    return Object.freeze({
      kind: "rejected" as const,
      reason: membershipRejectionReason(response.status),
    });
  }
  try {
    const fields = record(await boundedJson(response));
    const kind = fields["kind"];
    if (
      (kind !== "authenticated" && kind !== "released") ||
      text(fields["subscription_id"]) !== request.subscription.subscriptionId ||
      text(fields["descriptor_binding"]) !== request.subscription.descriptorBinding ||
      text(fields["stream"]) !== request.subscription.stream ||
      text(fields["control_nonce"]) !== request.controlNonce ||
      integer(fields["transport_generation"]) !== request.transportGeneration
    ) {
      return Object.freeze({ kind: "rejected" as const, reason: "closed" as const });
    }
  } catch {
    return Object.freeze({ kind: "rejected" as const, reason: "closed" as const });
  }
  return Object.freeze({
    connection: request.connection,
    controlNonce: request.controlNonce,
    descriptorBinding: request.subscription.descriptorBinding,
    kind: "authenticated" as const,
    operation: request.operation,
    stream: request.subscription.stream,
    subscriptionId: request.subscription.subscriptionId,
    transportGeneration: request.transportGeneration,
  });
}

function browserTimers(): AsyncTimerPort {
  return Object.freeze({
    clearTimeout: (handle: number) => {
      window.clearTimeout(handle);
    },
    timeout: (callback: VoidFunction, milliseconds: number) => {
      // suprnova-correctness-delay-allow: product-timer -- the runtime's reconnect, heartbeat, and poll policies are real product timers in the browser.
      return window.setTimeout(callback, milliseconds);
    },
  });
}

/**
 * Builds the complete feature configuration for a browser served by the
 * reserved Live routes: `configureAsync(browserAsyncOptions())`.
 */
export function browserAsyncOptions(options?: BrowserAsyncHostOptions): AsyncFeatureOptions {
  const host = resolveHost(options);
  const timers = browserTimers();
  return Object.freeze({
    authority: new BrowserAsyncAuthority(options),
    clock: Object.freeze({ now: () => Date.now() }),
    randomness: Object.freeze({ number: () => Math.random() }),
    timers,
    transports: new BrowserAsyncTransportPorts({
      eventSource: (url, init) => new EventSource(url, init),
      fetch: host.fetch,
      membershipTimeoutMs: MEMBERSHIP_TIMEOUT_MS,
      sseMembership: (request) => browserSseMembership(request, host.fetch),
      timers,
      webSocket: (url) => new WebSocket(url),
    }),
  });
}
