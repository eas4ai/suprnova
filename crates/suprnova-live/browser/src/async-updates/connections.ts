import { inspectAsyncEnvelopeSubscription } from "./envelope.js";
import {
  canonicalize,
  parseCanonicalJson,
  type CanonicalLimits,
  type JsonObject,
} from "../canonical.js";
import type {
  AsyncRandomness,
  AsyncReconnectPolicy,
  AsyncTimerPort,
  AsyncTransportAuthorization,
  AuthorizedLogicalSubscription,
  DocumentTransportKey,
  SubscriptionState,
} from "./types.js";

const MAX_LOGICAL_SUBSCRIPTIONS = 256;
const MAX_PENDING_HANDSHAKES_PER_ORIGIN = 1_024;
const MAX_SSE_RECORD_BYTES = 65_536;
const MAX_WEBSOCKET_ACK_BYTES = 512;
// Reserved versioned routes of the framework host: one SSE reader per document
// transport and one same-origin WebSocket per document transport.
const ASYNC_EVENTS_PATH = "/__live/v1/async/events";
const ASYNC_SOCKET_PATH = "/__live/v1/async/socket";
const WEBSOCKET_CONTROL_NONCE = /^[0-9a-z]{16}$/u;
const SSE_CONNECTION_BRAND = Symbol("suprnova.live.async.sse.connection");
const WEBSOCKET_ACK_LIMITS: CanonicalLimits = Object.freeze({
  maxBytes: MAX_WEBSOCKET_ACK_BYTES,
  maxDepth: 2,
  maxEntries: 6,
  maxStringBytes: 128,
});

export type DocumentTransportCloseReason =
  "page_suspended" | "document_retired" | "transport_replaced" | "subscription_empty";

export type DocumentTransportFailure =
  "authorization_lost" | "heartbeat_lost" | "protocol_invalid" | "transport_lost";

export interface DocumentTransportConnectRequest {
  readonly authorization: AsyncTransportAuthorization;
  readonly key: DocumentTransportKey;
  readonly transportGeneration: number;
  failed(reason: DocumentTransportFailure): void;
  message(encoded: string): void;
  opened(): void;
}

export interface DocumentTransportPort {
  subscribe(
    subscription: AuthorizedLogicalSubscription,
  ): DocumentMembershipOutcome | Promise<DocumentMembershipOutcome>;
  unsubscribe(subscriptionId: string): void;
  close(reason: DocumentTransportCloseReason): void;
}

export interface DocumentMembershipAcknowledgment {
  readonly descriptorBinding: string;
  readonly kind: "authenticated";
  readonly stream: string;
  readonly subscriptionId: string;
  readonly transportGeneration: number;
}

export interface DocumentMembershipRejection {
  readonly kind: "rejected";
  readonly reason: "authorization_lost" | "capacity" | "closed" | "timeout";
}

export type DocumentMembershipOutcome =
  DocumentMembershipAcknowledgment | DocumentMembershipRejection;

export interface SseConnectionHandle {
  readonly [SSE_CONNECTION_BRAND]: true;
}

export interface SseMembershipControlRequest {
  readonly connection: SseConnectionHandle;
  readonly controlNonce: string;
  readonly key: DocumentTransportKey;
  readonly operation: "subscribe" | "unsubscribe";
  readonly signal: AbortSignal;
  readonly subscription: AuthorizedLogicalSubscription;
  readonly transportGeneration: number;
}

export interface SseMembershipAcknowledgment extends DocumentMembershipAcknowledgment {
  readonly connection: SseConnectionHandle;
  readonly controlNonce: string;
  readonly operation: "subscribe" | "unsubscribe";
}

export type SseMembershipOutcome = SseMembershipAcknowledgment | DocumentMembershipRejection;

export type EventSourcePort = DocumentTransportPort;
export type WebSocketPort = DocumentTransportPort;

export interface AsyncTransportPorts {
  eventSource(connect: DocumentTransportConnectRequest): EventSourcePort;
  webSocket(connect: DocumentTransportConnectRequest): WebSocketPort;
}

export interface LogicalSubscriptionSink {
  envelope(encoded: string): void;
  reauthorize(
    prior: AuthorizedLogicalSubscription | null,
    signal: AbortSignal,
  ): ReauthorizedLogicalSubscription | Promise<ReauthorizedLogicalSubscription>;
  state(state: SubscriptionState): void;
}

export interface ReauthorizedLogicalSubscription {
  commit(): "committed" | "degraded" | "pending" | "stale";
  discard(): void;
  readonly proof: "authoritative_no_tail" | "complete_replay" | null;
  readonly subscription: AuthorizedLogicalSubscription;
}

export interface LogicalSubscriptionHandle {
  close(): void;
  continuityLost(): void;
  continuityProved(): void;
  heartbeatLost(): void;
  presentationFailed(): void;
}

export type DocumentAuthorizationSource = 0 | 1;

export interface DocumentAuthorizationScheduler {
  schedule<T>(
    source: DocumentAuthorizationSource,
    signal: AbortSignal,
    operation: () => Promise<T>,
  ): Promise<T>;
}

export interface DocumentConnectionPoolOptions {
  readonly authorizationScheduler?: DocumentAuthorizationScheduler;
  readonly handshakeScheduler: OriginHandshakeScheduler;
  readonly handshakeTimeoutMs?: number;
  readonly randomness: AsyncRandomness;
  readonly reauthorizationConcurrency?: number;
  readonly reauthorizationTimeoutMs?: number;
  readonly timers: AsyncTimerPort;
  readonly transports: AsyncTransportPorts;
}

interface NativeEventSourceLike {
  close(): void;
}

interface NativeWebSocketLike {
  close(code?: number, reason?: string): void;
  send(data: string): void;
}

export interface BrowserAsyncTransportOptions {
  readonly eventSource: (
    url: string,
    init: Readonly<{ withCredentials: true }>,
  ) => NativeEventSourceLike;
  readonly fetch: typeof globalThis.fetch;
  readonly membershipTimeoutMs: number;
  readonly sseMembership: (
    request: SseMembershipControlRequest,
  ) => SseMembershipOutcome | Promise<SseMembershipOutcome>;
  readonly timers: AsyncTimerPort;
  readonly webSocket: (url: string) => NativeWebSocketLike;
}

function setHandler(target: object, property: string, handler: (event?: unknown) => void): void {
  try {
    Reflect.set(target, property, handler);
  } catch {
    throw new Error("async_transport_invalid");
  }
}

function messageData(event: unknown): string | null {
  if ((typeof event !== "object" && typeof event !== "function") || event === null) return null;
  try {
    const data: unknown = Reflect.get(event, "data");
    return typeof data === "string" ? data : null;
  } catch {
    return null;
  }
}

function findRecordEnd(bytes: Uint8Array): number {
  for (let index = 0; index + 1 < bytes.byteLength; index += 1) {
    if (bytes[index] === 10 && bytes[index + 1] === 10) return index;
  }
  return -1;
}

function appendBytes(left: Uint8Array, right: Uint8Array): Uint8Array {
  if (right.byteLength > MAX_SSE_RECORD_BYTES + 512 - left.byteLength) {
    throw new Error("async_sse_record_too_large");
  }
  const joined = new Uint8Array(left.byteLength + right.byteLength);
  joined.set(left);
  joined.set(right, left.byteLength);
  return joined;
}

function decodeSseRecord(bytes: Uint8Array): string | null {
  if (bytes.byteLength === 0 || bytes[0] === 58) return null;
  let record: string;
  try {
    record = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error("async_sse_record_invalid");
  }
  let data: string | null = null;
  for (const line of record.split("\n")) {
    if (line.startsWith("data:")) {
      if (data !== null) throw new Error("async_sse_record_invalid");
      data = line.slice(5);
    } else if (
      line.length !== 0 &&
      !line.startsWith("id:") &&
      line !== "event:suprnova-live-async"
    ) {
      throw new Error("async_sse_record_invalid");
    }
  }
  if (data === null || new TextEncoder().encode(data).byteLength > MAX_SSE_RECORD_BYTES) {
    throw new Error("async_sse_record_invalid");
  }
  return data;
}

const MAX_ACTIVE_SSE_MEMBERSHIP_CONTROLS = 8;
const MAX_QUEUED_SSE_MEMBERSHIP_CONTROLS = MAX_LOGICAL_SUBSCRIPTIONS * 2;

interface PendingMembershipControl {
  abort: AbortController | null;
  readonly completion: MembershipControlCompletion;
  request: QueuedMembershipControl | null;
  settled: boolean;
  timer: number | null;
}

interface QueuedMembershipControl {
  readonly controlNonce: string;
  readonly operation: "subscribe" | "unsubscribe";
  resolve(outcome: DocumentMembershipOutcome): void;
  readonly subscription: AuthorizedLogicalSubscription;
}

interface MembershipControlCompletion {
  settle: ((outcome: MembershipControlSettlement) => void) | null;
}

type MembershipControlSettlement =
  | Readonly<{ acknowledgment: SseMembershipAcknowledgment; kind: "authenticated" }>
  | Readonly<{
      kind: "rejected";
      reason: DocumentMembershipRejection["reason"];
    }>;

function completeMembershipControl(
  completion: MembershipControlCompletion,
  outcome: MembershipControlSettlement,
): void {
  completion.settle?.(outcome);
}

function membershipRejection(
  reason: DocumentMembershipRejection["reason"],
): DocumentMembershipRejection {
  return Object.freeze({ kind: "rejected", reason });
}

function membershipRejectionReason(outcome: unknown): DocumentMembershipRejection["reason"] | null {
  if ((typeof outcome !== "object" && typeof outcome !== "function") || outcome === null) {
    return null;
  }
  try {
    const reason: unknown = Reflect.get(outcome, "reason");
    return Reflect.get(outcome, "kind") === "rejected" &&
      (reason === "authorization_lost" ||
        reason === "capacity" ||
        reason === "closed" ||
        reason === "timeout")
      ? reason
      : null;
  } catch {
    return null;
  }
}

function sseConnectionHandle(): SseConnectionHandle {
  return Object.freeze({ [SSE_CONNECTION_BRAND]: true as const });
}

function validateTransportGeneration(request: DocumentTransportConnectRequest): void {
  if (!Number.isSafeInteger(request.transportGeneration) || request.transportGeneration < 1) {
    throw new Error("async_transport_generation_invalid");
  }
}

class SseMembershipControls {
  readonly #connection: SseConnectionHandle;
  readonly #membership: BrowserAsyncTransportOptions["sseMembership"];
  readonly #pending = new Set<PendingMembershipControl>();
  readonly #queue: QueuedMembershipControl[] = [];
  readonly #request: DocumentTransportConnectRequest;
  readonly #timeoutMs: number;
  readonly #timers: AsyncTimerPort;
  #closed = false;
  #nextControl = 0;

  constructor(
    request: DocumentTransportConnectRequest,
    membership: BrowserAsyncTransportOptions["sseMembership"],
    timers: AsyncTimerPort,
    timeoutMs: number,
    connection: SseConnectionHandle,
  ) {
    this.#request = request;
    this.#membership = membership;
    this.#timers = timers;
    this.#timeoutMs = timeoutMs;
    this.#connection = connection;
  }

  request(
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
  ): Promise<DocumentMembershipOutcome> {
    return new Promise((resolve) => {
      if (this.#closed) {
        resolve(membershipRejection("closed"));
        return;
      }
      if (this.#pending.size + this.#queue.length >= MAX_QUEUED_SSE_MEMBERSHIP_CONTROLS) {
        resolve(membershipRejection("capacity"));
        if (operation === "subscribe") this.#request.failed("authorization_lost");
        return;
      }
      this.#nextControl += 1;
      this.#queue.push({
        controlNonce: this.#nextControl.toString(36).padStart(16, "0"),
        operation,
        resolve,
        subscription,
      });
      this.#pump();
    });
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    for (const request of this.#queue.splice(0)) {
      request.resolve(membershipRejection("closed"));
    }
    for (const control of [...this.#pending]) {
      control.abort?.abort();
      this.#settle(control, { kind: "rejected", reason: "closed" });
    }
  }

  #start(request: QueuedMembershipControl): void {
    const completion: MembershipControlCompletion = { settle: null };
    const control: PendingMembershipControl = {
      abort: new AbortController(),
      completion,
      request,
      settled: false,
      timer: null,
    };
    completion.settle = (outcome) => {
      this.#settle(control, outcome);
    };
    this.#pending.add(control);
    control.timer = this.#timers.timeout(() => {
      control.abort?.abort();
      completeMembershipControl(completion, { kind: "rejected", reason: "timeout" });
    }, this.#timeoutMs);
    let pending: unknown;
    try {
      const abort = control.abort;
      if (abort === null) return;
      pending = this.#membership(
        Object.freeze({
          connection: this.#connection,
          controlNonce: request.controlNonce,
          key: this.#request.key,
          operation: request.operation,
          signal: abort.signal,
          subscription: request.subscription,
          transportGeneration: this.#request.transportGeneration,
        }),
      );
    } catch {
      completeMembershipControl(completion, {
        kind: "rejected",
        reason: "authorization_lost",
      });
      return;
    }
    void Promise.resolve(pending).then(
      (outcome) => {
        if (this.#validAcknowledgment(outcome, request)) {
          completeMembershipControl(completion, { acknowledgment: outcome, kind: "authenticated" });
        } else {
          completeMembershipControl(completion, {
            kind: "rejected",
            reason: membershipRejectionReason(outcome) ?? "authorization_lost",
          });
        }
      },
      () => {
        completeMembershipControl(completion, {
          kind: "rejected",
          reason: "authorization_lost",
        });
      },
    );
  }

  #pump(): void {
    while (
      !this.#closed &&
      this.#pending.size < MAX_ACTIVE_SSE_MEMBERSHIP_CONTROLS &&
      this.#queue.length !== 0
    ) {
      const request = this.#queue.shift();
      if (request !== undefined) this.#start(request);
    }
  }

  #settle(control: PendingMembershipControl, outcome: MembershipControlSettlement): void {
    if (control.settled) return;
    control.settled = true;
    control.completion.settle = null;
    if (control.timer !== null) this.#timers.clearTimeout(control.timer);
    control.timer = null;
    control.abort = null;
    const request = control.request;
    control.request = null;
    this.#pending.delete(control);
    if (request !== null) {
      if (outcome.kind === "authenticated") {
        request.resolve(outcome.acknowledgment);
      } else {
        request.resolve(membershipRejection(outcome.reason));
      }
    }
    if (outcome.kind === "rejected" && request?.operation === "subscribe" && !this.#closed) {
      this.#request.failed("authorization_lost");
      return;
    }
    this.#pump();
  }

  #validAcknowledgment(
    outcome: unknown,
    request: QueuedMembershipControl,
  ): outcome is SseMembershipAcknowledgment {
    if ((typeof outcome !== "object" && typeof outcome !== "function") || outcome === null) {
      return false;
    }
    try {
      return (
        Reflect.get(outcome, "kind") === "authenticated" &&
        Reflect.get(outcome, "connection") === this.#connection &&
        Reflect.get(outcome, "controlNonce") === request.controlNonce &&
        Reflect.get(outcome, "operation") === request.operation &&
        Reflect.get(outcome, "subscriptionId") === request.subscription.subscriptionId &&
        Reflect.get(outcome, "descriptorBinding") === request.subscription.descriptorBinding &&
        Reflect.get(outcome, "stream") === request.subscription.stream &&
        Reflect.get(outcome, "transportGeneration") === this.#request.transportGeneration
      );
    } catch {
      return false;
    }
  }
}

class NativeEventSourceAdapter implements EventSourcePort {
  readonly #controls: SseMembershipControls;
  readonly #native: NativeEventSourceLike;
  readonly #subscriptions = new Map<string, AuthorizedLogicalSubscription>();
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    create: BrowserAsyncTransportOptions["eventSource"],
    membership: BrowserAsyncTransportOptions["sseMembership"],
    timers: AsyncTimerPort,
    membershipTimeoutMs: number,
  ) {
    validateTransportGeneration(request);
    this.#controls = new SseMembershipControls(
      request,
      membership,
      timers,
      membershipTimeoutMs,
      sseConnectionHandle(),
    );
    const url = new URL(ASYNC_EVENTS_PATH, request.key.origin).href;
    this.#native = create(url, Object.freeze({ withCredentials: true }));
    setHandler(this.#native, "onopen", () => {
      if (!this.#closed) request.opened();
    });
    setHandler(this.#native, "onmessage", (event) => {
      if (this.#closed) return;
      const data = messageData(event);
      if (data === null) request.failed("protocol_invalid");
      else request.message(data);
    });
    setHandler(this.#native, "onerror", () => {
      if (!this.#closed) request.failed("transport_lost");
    });
  }

  subscribe(subscription: AuthorizedLogicalSubscription): Promise<DocumentMembershipOutcome> {
    if (this.#closed) return Promise.resolve(membershipRejection("closed"));
    this.#subscriptions.set(subscription.subscriptionId, subscription);
    return this.#controls.request("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    void this.#controls.request("unsubscribe", subscription).catch(() => undefined);
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#controls.close();
    this.#subscriptions.clear();
    this.#native.close();
  }
}

class FetchEventSourceAdapter implements EventSourcePort {
  readonly #abort = new AbortController();
  readonly #controls: SseMembershipControls;
  readonly #request: DocumentTransportConnectRequest;
  readonly #subscriptions = new Map<string, AuthorizedLogicalSubscription>();
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    fetchPort: typeof globalThis.fetch,
    membership: BrowserAsyncTransportOptions["sseMembership"],
    timers: AsyncTimerPort,
    membershipTimeoutMs: number,
  ) {
    validateTransportGeneration(request);
    this.#request = request;
    this.#controls = new SseMembershipControls(
      request,
      membership,
      timers,
      membershipTimeoutMs,
      sseConnectionHandle(),
    );
    const authorization = request.authorization;
    if (authorization.kind !== "bearer" || authorization.credential.length === 0) {
      throw new Error("async_transport_authorization_invalid");
    }
    const url = new URL(ASYNC_EVENTS_PATH, request.key.origin);
    const headers = new Headers({
      Accept: "text/event-stream",
      Authorization: `SuprnovaAsync ${authorization.credential}`,
      "Suprnova-Transport-Generation": String(request.transportGeneration),
    });
    void this.#read(fetchPort, url, headers);
  }

  subscribe(subscription: AuthorizedLogicalSubscription): Promise<DocumentMembershipOutcome> {
    if (this.#closed) return Promise.resolve(membershipRejection("closed"));
    this.#subscriptions.set(subscription.subscriptionId, subscription);
    return this.#controls.request("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    void this.#controls.request("unsubscribe", subscription).catch(() => undefined);
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#subscriptions.clear();
    this.#controls.close();
    this.#abort.abort();
  }

  async #read(fetchPort: typeof globalThis.fetch, url: URL, headers: Headers): Promise<void> {
    try {
      const response = await fetchPort(url.href, {
        cache: "no-store",
        // Same-origin identity travels on the cookie so the host can re-resolve
        // session, principal, and tenant before matching the bearer credential;
        // a cross-origin document transport sends no ambient credential.
        credentials: "same-origin",
        headers,
        method: "GET",
        redirect: "error",
        referrerPolicy: "no-referrer",
        signal: this.#abort.signal,
      });
      if (
        this.#closed ||
        !response.ok ||
        !response.headers.get("Content-Type")?.toLowerCase().startsWith("text/event-stream")
      ) {
        if (!this.#closed) this.#request.failed("transport_lost");
        return;
      }
      const reader = response.body?.getReader();
      if (reader === undefined) {
        this.#request.failed("transport_lost");
        return;
      }
      this.#request.opened();
      let buffered: Uint8Array = new Uint8Array(0);
      for (;;) {
        const item = await reader.read();
        // The owned adapter can be closed while the awaited stream read is pending.
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- async lifecycle reentry mutates closed
        if (this.#closed) {
          await reader.cancel();
          return;
        }
        if (item.done) {
          if (buffered.byteLength !== 0) throw new Error("async_sse_record_invalid");
          this.#request.failed("transport_lost");
          return;
        }
        buffered = appendBytes(buffered, item.value);
        for (;;) {
          const end = findRecordEnd(buffered);
          if (end < 0) break;
          if (end > MAX_SSE_RECORD_BYTES) throw new Error("async_sse_record_too_large");
          const data = decodeSseRecord(buffered.slice(0, end));
          buffered = buffered.slice(end + 2);
          if (data !== null) this.#request.message(data);
        }
        if (buffered.byteLength > MAX_SSE_RECORD_BYTES + 512) {
          throw new Error("async_sse_record_too_large");
        }
      }
    } catch {
      if (!this.#closed && !this.#abort.signal.aborted) this.#request.failed("protocol_invalid");
    }
  }
}

class BrowserWebSocketAdapter implements WebSocketPort {
  readonly #native: NativeWebSocketLike;
  readonly #pending = new Map<string, PendingWebSocketMembership>();
  readonly #request: DocumentTransportConnectRequest;
  readonly #timeoutMs: number;
  readonly #timers: AsyncTimerPort;
  #closed = false;
  #nextControl = 0;

  constructor(
    request: DocumentTransportConnectRequest,
    create: BrowserAsyncTransportOptions["webSocket"],
    timers: AsyncTimerPort,
    timeoutMs: number,
  ) {
    validateTransportGeneration(request);
    if (request.authorization.kind !== "session_cookie") {
      throw new Error("async_transport_authorization_invalid");
    }
    this.#request = request;
    this.#timers = timers;
    this.#timeoutMs = timeoutMs;
    const url = new URL(ASYNC_SOCKET_PATH, request.key.origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    this.#native = create(url.href);
    setHandler(this.#native, "onopen", () => {
      if (!this.#closed) request.opened();
    });
    setHandler(this.#native, "onmessage", (event) => {
      if (this.#closed) return;
      const data = messageData(event);
      if (data === null) request.failed("protocol_invalid");
      else if (!this.#membershipAcknowledged(data)) request.message(data);
    });
    // A WebSocket `error` event is always followed by `close`, and only `close`
    // carries the authoritative code and reason, so classification waits for it.
    setHandler(this.#native, "onerror", () => undefined);
    setHandler(this.#native, "onclose", () => {
      if (!this.#closed) request.failed("transport_lost");
    });
  }

  subscribe(subscription: AuthorizedLogicalSubscription): Promise<DocumentMembershipOutcome> {
    return new Promise((resolve) => {
      if (this.#closed || this.#pending.size >= MAX_LOGICAL_SUBSCRIPTIONS) {
        resolve(membershipRejection(this.#closed ? "closed" : "capacity"));
        if (!this.#closed) this.#request.failed("authorization_lost");
        return;
      }
      this.#nextControl += 1;
      const controlNonce = this.#nextControl.toString(36).padStart(16, "0");
      const pending: PendingWebSocketMembership = {
        descriptorBinding: subscription.descriptorBinding,
        resolve,
        stream: subscription.stream,
        subscriptionId: subscription.subscriptionId,
        timer: null,
      };
      this.#pending.set(controlNonce, pending);
      pending.timer = this.#timers.timeout(() => {
        this.#settleMembership(controlNonce, membershipRejection("timeout"));
      }, this.#timeoutMs);
      try {
        this.#native.send(
          JSON.stringify({
            control_nonce: controlNonce,
            descriptor_binding: subscription.descriptorBinding,
            kind: "subscribe",
            stream: subscription.stream,
            subscription: subscription.subscriptionId,
            transport_generation: this.#request.transportGeneration,
          }),
        );
      } catch {
        this.#settleMembership(controlNonce, membershipRejection("authorization_lost"));
      }
    });
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    this.#native.send(JSON.stringify({ kind: "unsubscribe", subscription: subscriptionId }));
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    for (const controlNonce of [...this.#pending.keys()]) {
      this.#settleMembership(controlNonce, membershipRejection("closed"));
    }
    this.#native.close(1000, "suprnova_live_async_closed");
  }

  #membershipAcknowledged(encoded: string): boolean {
    if (new TextEncoder().encode(encoded).byteLength > MAX_WEBSOCKET_ACK_BYTES) return false;
    let candidate: unknown;
    try {
      candidate = JSON.parse(encoded);
    } catch {
      return false;
    }
    if ((typeof candidate !== "object" && typeof candidate !== "function") || candidate === null) {
      return false;
    }
    let kind: unknown;
    try {
      kind = Reflect.get(candidate, "kind");
    } catch {
      return false;
    }
    if (kind !== "membership_authenticated") return false;
    let value: JsonObject;
    try {
      const parsed = parseCanonicalJson(encoded, WEBSOCKET_ACK_LIMITS);
      if (
        parsed === null ||
        typeof parsed !== "object" ||
        Array.isArray(parsed) ||
        canonicalize(parsed) !== encoded
      ) {
        throw new Error("async_websocket_membership_ack_invalid");
      }
      value = parsed as JsonObject;
    } catch {
      this.#request.failed("protocol_invalid");
      return true;
    }
    let controlNonce: unknown;
    let descriptorBinding: unknown;
    let stream: unknown;
    let subscriptionId: unknown;
    try {
      const keys = Object.keys(value).sort();
      if (
        keys.length !== 6 ||
        keys[0] !== "control_nonce" ||
        keys[1] !== "descriptor_binding" ||
        keys[2] !== "kind" ||
        keys[3] !== "stream" ||
        keys[4] !== "subscription" ||
        keys[5] !== "transport_generation"
      ) {
        throw new Error("async_websocket_membership_ack_invalid");
      }
      controlNonce = Reflect.get(value, "control_nonce");
      descriptorBinding = Reflect.get(value, "descriptor_binding");
      stream = Reflect.get(value, "stream");
      subscriptionId = Reflect.get(value, "subscription");
    } catch {
      this.#request.failed("protocol_invalid");
      return true;
    }
    if (typeof controlNonce !== "string" || !WEBSOCKET_CONTROL_NONCE.test(controlNonce)) {
      this.#request.failed("protocol_invalid");
      return true;
    }
    const pending = this.#pending.get(controlNonce);
    if (
      pending === undefined ||
      pending.descriptorBinding !== descriptorBinding ||
      pending.stream !== stream ||
      pending.subscriptionId !== subscriptionId ||
      Reflect.get(value, "transport_generation") !== this.#request.transportGeneration
    ) {
      this.#request.failed("protocol_invalid");
      return true;
    }
    this.#settleMembership(
      controlNonce,
      Object.freeze({
        descriptorBinding: pending.descriptorBinding,
        kind: "authenticated",
        stream: pending.stream,
        subscriptionId: pending.subscriptionId,
        transportGeneration: this.#request.transportGeneration,
      }),
    );
    return true;
  }

  #settleMembership(controlNonce: string, outcome: DocumentMembershipOutcome): void {
    const pending = this.#pending.get(controlNonce);
    if (pending === undefined) return;
    this.#pending.delete(controlNonce);
    if (pending.timer !== null) this.#timers.clearTimeout(pending.timer);
    pending.timer = null;
    if (outcome.kind === "rejected") {
      pending.resolve(outcome);
      if (!this.#closed) this.#request.failed("authorization_lost");
    } else {
      pending.resolve(outcome);
    }
  }
}

interface PendingWebSocketMembership {
  readonly descriptorBinding: string;
  resolve(outcome: DocumentMembershipOutcome): void;
  readonly stream: string;
  readonly subscriptionId: string;
  timer: number | null;
}

export class BrowserAsyncTransportPorts implements AsyncTransportPorts {
  readonly #options: BrowserAsyncTransportOptions;

  constructor(options: BrowserAsyncTransportOptions) {
    if (
      typeof options.eventSource !== "function" ||
      typeof options.fetch !== "function" ||
      !Number.isSafeInteger(options.membershipTimeoutMs) ||
      options.membershipTimeoutMs < 1 ||
      options.membershipTimeoutMs > 30_000 ||
      typeof options.sseMembership !== "function" ||
      typeof options.timers.clearTimeout !== "function" ||
      typeof options.timers.timeout !== "function" ||
      typeof options.webSocket !== "function"
    ) {
      throw new TypeError("async_transport_configuration_invalid");
    }
    this.#options = options;
  }

  eventSource(connect: DocumentTransportConnectRequest): EventSourcePort {
    return connect.authorization.kind === "session_cookie"
      ? new NativeEventSourceAdapter(
          connect,
          this.#options.eventSource,
          this.#options.sseMembership,
          this.#options.timers,
          this.#options.membershipTimeoutMs,
        )
      : new FetchEventSourceAdapter(
          connect,
          this.#options.fetch,
          this.#options.sseMembership,
          this.#options.timers,
          this.#options.membershipTimeoutMs,
        );
  }

  webSocket(connect: DocumentTransportConnectRequest): WebSocketPort {
    return new BrowserWebSocketAdapter(
      connect,
      this.#options.webSocket,
      this.#options.timers,
      this.#options.membershipTimeoutMs,
    );
  }
}

interface HandshakeRecord {
  active: boolean;
  canceled: boolean;
  readonly start: (release: VoidFunction) => void;
}

interface OriginHandshakes {
  active: number;
  queue: HandshakeRecord[];
}

export interface HandshakeRequest {
  cancel(): void;
}

export class OriginHandshakeScheduler {
  readonly #maximum: number;
  readonly #origins = new Map<string, OriginHandshakes>();

  constructor(maximum = 8) {
    if (!Number.isSafeInteger(maximum) || maximum < 1 || maximum > 8) {
      throw new RangeError("async_handshake_limit_invalid");
    }
    this.#maximum = maximum;
  }

  schedule(origin: string, start: (release: VoidFunction) => void): HandshakeRequest {
    if (typeof start !== "function") throw new TypeError("async_handshake_start_invalid");
    const state = this.#origins.get(origin) ?? { active: 0, queue: [] };
    if (!this.#origins.has(origin)) this.#origins.set(origin, state);
    if (state.queue.length >= MAX_PENDING_HANDSHAKES_PER_ORIGIN) {
      throw new Error("async_handshake_queue_full");
    }
    const record: HandshakeRecord = { active: false, canceled: false, start };
    state.queue.push(record);
    this.#pump(origin, state);
    return Object.freeze({
      cancel: () => {
        if (record.canceled) return;
        record.canceled = true;
        if (record.active) this.#release(origin, state, record);
      },
    });
  }

  active(origin: string): number {
    return this.#origins.get(origin)?.active ?? 0;
  }

  #pump(origin: string, state: OriginHandshakes): void {
    while (state.active < this.#maximum) {
      const record = state.queue.shift();
      if (record === undefined) break;
      if (record.canceled) continue;
      record.active = true;
      state.active += 1;
      let released = false;
      const release = () => {
        if (released) return;
        released = true;
        this.#release(origin, state, record);
      };
      try {
        record.start(release);
      } catch {
        release();
      }
    }
    this.#prune(origin, state);
  }

  #release(origin: string, state: OriginHandshakes, record: HandshakeRecord): void {
    if (!record.active) return;
    record.active = false;
    state.active -= 1;
    this.#pump(origin, state);
  }

  #prune(origin: string, state: OriginHandshakes): void {
    if (state.active === 0 && state.queue.length === 0) this.#origins.delete(origin);
  }
}

interface LogicalMembership {
  attachmentCompletion: MembershipAttachmentCompletion | null;
  authorization: AuthorizedLogicalSubscription;
  authenticatedTransportGeneration: number;
  readonly sink: LogicalSubscriptionSink;
  group: PhysicalGroup | null;
  active: boolean;
  generation: number;
  pendingProofMembershipGeneration: number;
  pendingObservedTransportGeneration: number;
  pendingAuthorization: ReauthorizedLogicalSubscription | null;
  pendingAuthorizationKind: "initial" | "successor" | null;
  provedTransportGeneration: number;
  quarantinedDescriptorBinding: string | null;
  quarantinedGroup: PhysicalGroup | null;
  quarantinedTransportGeneration: number;
  recoveryRequestGeneration: number;
  requiresInitialAuthorization: boolean;
  logicallyDegraded: boolean;
}

interface PhysicalGroup {
  authorization: AsyncTransportAuthorization;
  continuityTransportGeneration: number;
  readonly key: DocumentTransportKey;
  readonly keyValue: string;
  readonly memberships: Map<string, LogicalMembership>;
  readonly recovering: Map<string, LogicalMembership>;
  recoveryActive: boolean;
  recoveryGeneration: number;
  policy: AsyncReconnectPolicy;
  generation: number;
  port: DocumentTransportPort | null;
  handshake: HandshakeRequest | null;
  handshakeTimer: number | null;
  releaseHandshake: VoidFunction | null;
  reconnectAttempt: number;
  reconnectTimer: number | null;
  state: "idle" | "backoff" | "restoring" | "connecting" | "open" | "closed";
}

interface ReauthorizationRequest {
  abort: AbortController | null;
  readonly completion: ReauthorizationCompletion;
  generation: number;
  membership: LogicalMembership | null;
  membershipGeneration: number;
  prior: AuthorizedLogicalSubscription | null;
  readonly result: Promise<ReauthorizedLogicalSubscription | null>;
  resolve(value: ReauthorizedLogicalSubscription | null): void;
  settled: boolean;
  timer: number | null;
}

interface ReauthorizationCompletion {
  settle: ((current: ReauthorizedLogicalSubscription | null) => void) | null;
}

interface MembershipAttachmentCompletion {
  settle: ((acknowledgment: unknown) => void) | null;
}

function completeMembershipAttachment(
  completion: MembershipAttachmentCompletion,
  acknowledgment: unknown,
): void {
  completion.settle?.(acknowledgment);
}

function completeReauthorization(
  completion: ReauthorizationCompletion,
  current: ReauthorizedLogicalSubscription | null,
): void {
  completion.settle?.(current);
}

function discardReauthorization(current: ReauthorizedLogicalSubscription | null): void {
  try {
    current?.discard();
  } catch {
    // A rejected stage is inert even when its cleanup callback is hostile.
  }
}

function normalizedKey(key: DocumentTransportKey): DocumentTransportKey {
  const transport: unknown = key.transport;
  let url: URL;
  try {
    url = new URL(key.origin);
  } catch {
    throw new Error("async_document_transport_key_invalid");
  }
  if (
    url.origin !== key.origin ||
    (url.protocol !== "https:" && url.protocol !== "http:") ||
    key.authorizationScope.length < 1 ||
    key.authorizationScope.length > 256 ||
    (transport !== "sse" && transport !== "websocket")
  ) {
    throw new Error("async_document_transport_key_invalid");
  }
  return Object.freeze({
    authorizationScope: key.authorizationScope,
    origin: url.origin,
    transport,
  });
}

function keyValue(key: DocumentTransportKey): string {
  return JSON.stringify([key.origin, key.transport, key.authorizationScope]);
}

function validRandom(value: number): number {
  if (!Number.isFinite(value) || value < 0 || value >= 1) {
    throw new Error("async_randomness_invalid");
  }
  return value;
}

function sameAuthorization(
  left: AsyncTransportAuthorization,
  right: AsyncTransportAuthorization,
): boolean {
  return (
    (left.kind === "session_cookie" && right.kind === "session_cookie") ||
    (left.kind === "bearer" && right.kind === "bearer" && left.credential === right.credential)
  );
}

function sameKey(left: DocumentTransportKey, right: DocumentTransportKey): boolean {
  return keyValue(normalizedKey(left)) === keyValue(normalizedKey(right));
}

function aggregatePolicy(
  subscriptions: readonly AuthorizedLogicalSubscription[],
): AsyncReconnectPolicy {
  const maximumAttempts = Math.min(
    ...subscriptions.map(({ reconnect }) => reconnect.maximumAttempts),
  );
  const minimumDelayMs = Math.max(
    ...subscriptions.map(({ reconnect }) => reconnect.minimumDelayMs),
  );
  const maximumDelayMs = Math.min(
    ...subscriptions.map(({ reconnect }) => reconnect.maximumDelayMs),
  );
  if (
    subscriptions.length === 0 ||
    !Number.isSafeInteger(maximumAttempts) ||
    maximumAttempts < 1 ||
    !Number.isSafeInteger(minimumDelayMs) ||
    minimumDelayMs < 1 ||
    !Number.isSafeInteger(maximumDelayMs) ||
    maximumDelayMs < minimumDelayMs
  ) {
    throw new Error("async_transport_policy_conflict");
  }
  const kind = subscriptions.some(({ reconnect }) => reconnect.kind === "refresh_on_reconnect")
    ? "refresh_on_reconnect"
    : "resume_or_refresh";
  return Object.freeze({ kind, maximumAttempts, maximumDelayMs, minimumDelayMs });
}

function commonAuthorization(
  subscriptions: readonly AuthorizedLogicalSubscription[],
): AsyncTransportAuthorization {
  const authority = subscriptions[0]?.authorization;
  if (
    authority === undefined ||
    subscriptions.some(({ authorization }) => !sameAuthorization(authority, authorization))
  ) {
    throw new Error("async_transport_authority_conflict");
  }
  return authority;
}

export class DocumentConnectionPool {
  readonly #authorizationScheduler: DocumentAuthorizationScheduler | null;
  readonly #handshakes: OriginHandshakeScheduler;
  readonly #handshakeTimeoutMs: number;
  readonly #randomness: AsyncRandomness;
  readonly #reauthorizationConcurrency: number;
  readonly #reauthorizationTimeoutMs: number;
  readonly #timers: AsyncTimerPort;
  readonly #transports: AsyncTransportPorts;
  readonly #groups = new Map<string, PhysicalGroup>();
  readonly #memberships = new Map<string, LogicalMembership>();
  readonly #reauthorizationQueue: ReauthorizationRequest[] = [];
  readonly #activeReauthorizations = new Set<ReauthorizationRequest>();
  #generation = 0;
  #state: "active" | "suspended" | "resuming" | "retired" = "active";

  constructor(options: DocumentConnectionPoolOptions) {
    const concurrency = options.reauthorizationConcurrency ?? 8;
    const timeout = options.reauthorizationTimeoutMs ?? 5_000;
    const handshakeTimeout = options.handshakeTimeoutMs ?? 5_000;
    if (!Number.isSafeInteger(concurrency) || concurrency < 1 || concurrency > 8) {
      throw new RangeError("async_reauthorization_concurrency_invalid");
    }
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30_000) {
      throw new RangeError("async_reauthorization_timeout_invalid");
    }
    if (
      !Number.isSafeInteger(handshakeTimeout) ||
      handshakeTimeout < 1 ||
      handshakeTimeout > 30_000
    ) {
      throw new RangeError("async_handshake_timeout_invalid");
    }
    this.#authorizationScheduler = options.authorizationScheduler ?? null;
    this.#handshakes = options.handshakeScheduler;
    this.#handshakeTimeoutMs = handshakeTimeout;
    this.#randomness = options.randomness;
    this.#reauthorizationConcurrency = concurrency;
    this.#reauthorizationTimeoutMs = timeout;
    this.#timers = options.timers;
    this.#transports = options.transports;
  }

  subscribe(
    authorization: AuthorizedLogicalSubscription,
    sink: LogicalSubscriptionSink,
    pendingAuthorization: ReauthorizedLogicalSubscription | null = null,
  ): LogicalSubscriptionHandle {
    if (this.#state === "retired") throw new Error("async_document_retired");
    if (this.#memberships.size >= MAX_LOGICAL_SUBSCRIPTIONS) {
      throw new Error("async_subscription_limit");
    }
    if (this.#memberships.has(authorization.subscriptionId)) {
      throw new Error("async_subscription_duplicate");
    }
    if (pendingAuthorization !== null && pendingAuthorization.subscription !== authorization) {
      throw new Error("async_subscription_stage_invalid");
    }
    const membership: LogicalMembership = {
      active: true,
      attachmentCompletion: null,
      authorization,
      authenticatedTransportGeneration: -1,
      generation: 0,
      group: null,
      pendingProofMembershipGeneration: -1,
      pendingObservedTransportGeneration: -1,
      pendingAuthorization,
      pendingAuthorizationKind: pendingAuthorization === null ? null : "initial",
      provedTransportGeneration: -1,
      quarantinedDescriptorBinding: null,
      quarantinedGroup: null,
      quarantinedTransportGeneration: -1,
      recoveryRequestGeneration: -1,
      requiresInitialAuthorization: pendingAuthorization !== null,
      logicallyDegraded: false,
      sink,
    };
    this.#memberships.set(authorization.subscriptionId, membership);
    try {
      if (this.#transportAvailable()) this.#attach(membership);
    } catch (error: unknown) {
      membership.active = false;
      membership.generation += 1;
      this.#memberships.delete(authorization.subscriptionId);
      throw error;
    }
    return Object.freeze({
      close: () => {
        this.#remove(membership);
      },
      continuityProved: () => {
        const group = membership.group;
        if (group !== null) this.#proveContinuity(group, membership, group.generation);
      },
      continuityLost: () => {
        const group = membership.group;
        if (
          membership.active &&
          group !== null &&
          (group.state === "open" || group.state === "connecting")
        ) {
          this.#failed(group, group.generation, "transport_lost");
        }
      },
      heartbeatLost: () => {
        const group = membership.group;
        if (
          membership.active &&
          group !== null &&
          (group.state === "open" || group.state === "connecting")
        ) {
          this.#failed(group, group.generation, "heartbeat_lost");
        }
      },
      presentationFailed: () => {
        this.#degradeLogicalMembership(membership);
      },
    });
  }

  suspend(): void {
    if (this.#state === "retired") return;
    if (this.#state === "suspended") {
      this.#generation += 1;
      this.#cancelReauthorizations();
      return;
    }
    this.#state = "suspended";
    this.#generation += 1;
    this.#cancelReauthorizations();
    for (const group of this.#groups.values()) this.#closeGroup(group, "page_suspended");
    this.#groups.clear();
    for (const membership of this.#memberships.values()) {
      this.#cancelMembershipAttachment(membership);
      this.#discardPendingAuthorization(membership);
      this.#clearMembershipQuarantine(membership);
      membership.group = null;
      membership.authenticatedTransportGeneration = -1;
      membership.pendingProofMembershipGeneration = -1;
      membership.pendingObservedTransportGeneration = -1;
      membership.provedTransportGeneration = -1;
      this.#safeState(membership, "disconnected");
    }
  }

  async resume(): Promise<void> {
    if (this.#state !== "suspended") return;
    const generation = ++this.#generation;
    this.#state = "resuming";
    const memberships = [...this.#memberships.values()].filter(({ active }) => active);
    await Promise.all(
      memberships.map(async (membership) => {
        const initial = membership.requiresInitialAuthorization;
        const current = await this.#requestReauthorization(membership, generation, initial);
        if (!this.#resumeCurrent(generation)) return;
        const staged =
          current === null ? null : this.#acceptedReauthorization(membership, current, initial);
        const accepted = staged?.subscription ?? null;
        if (accepted === null || staged === null) {
          discardReauthorization(current);
          if (membership.active) this.#safeState(membership, "degraded");
          return;
        }
        membership.pendingAuthorization = staged;
        membership.pendingAuthorizationKind = initial ? "initial" : "successor";
        try {
          this.#attach(membership);
        } catch {
          this.#discardPendingAuthorization(membership);
          this.#safeState(membership, "degraded");
        }
      }),
    );
    if (this.#resumeCurrent(generation)) this.#state = "active";
  }

  dispose(): void {
    if (this.#state === "retired") return;
    this.#state = "retired";
    this.#generation += 1;
    this.#cancelReauthorizations();
    for (const group of this.#groups.values()) this.#closeGroup(group, "document_retired");
    this.#groups.clear();
    for (const membership of this.#memberships.values()) {
      membership.active = false;
      this.#discardPendingAuthorization(membership);
      this.#clearMembershipQuarantine(membership);
      membership.group = null;
      this.#safeState(membership, "closed");
    }
    this.#memberships.clear();
  }

  #resumeCurrent(generation: number): boolean {
    return this.#state === "resuming" && this.#generation === generation;
  }

  #attach(membership: LogicalMembership): void {
    const effective = this.#effectiveAuthorization(membership);
    const key = normalizedKey(effective.document);
    const encodedKey = keyValue(key);
    let group = this.#groups.get(encodedKey);
    if (group === undefined) {
      group = {
        authorization: effective.authorization,
        continuityTransportGeneration: -1,
        generation: 0,
        handshake: null,
        handshakeTimer: null,
        key,
        keyValue: encodedKey,
        memberships: new Map(),
        policy: aggregatePolicy([effective]),
        port: null,
        recovering: new Map(),
        recoveryActive: false,
        recoveryGeneration: 0,
        reconnectAttempt: 0,
        reconnectTimer: null,
        releaseHandshake: null,
        state: "idle",
      };
      this.#groups.set(encodedKey, group);
    } else {
      const readySubscriptions = [...group.memberships.values()].map((candidate) =>
        this.#effectiveAuthorization(candidate),
      );
      const subscriptions = [
        ...readySubscriptions,
        ...[...group.recovering.values()].map((candidate) =>
          this.#effectiveAuthorization(candidate),
        ),
        effective,
      ];
      try {
        if (group.recoveryActive && readySubscriptions.length === 0) {
          if (!sameAuthorization(group.authorization, effective.authorization)) {
            throw new Error("async_transport_authority_conflict");
          }
        } else {
          group.authorization = commonAuthorization([...readySubscriptions, effective]);
        }
        group.policy = aggregatePolicy(subscriptions);
      } catch (error: unknown) {
        this.#retireConflictedGroup(group);
        throw error;
      }
    }
    if (group.recoveryActive) {
      group.recovering.set(effective.subscriptionId, membership);
      membership.group = group;
      this.#safeState(membership, "connecting");
      if (group.state !== "backoff") {
        this.#restoreMembership(group, membership, group.recoveryGeneration);
      }
      return;
    }
    group.memberships.set(effective.subscriptionId, membership);
    membership.group = group;
    this.#safeState(membership, "connecting");
    if (group.state === "open") {
      this.#subscribeMembership(group, membership, group.generation);
    } else if (group.state === "idle") {
      this.#connect(group, "idle");
    }
  }

  #connect(group: PhysicalGroup, expectedState: "idle" | "restoring"): void {
    if (
      !this.#transportAvailable() ||
      group.state !== expectedState ||
      group.memberships.size === 0
    ) {
      return;
    }
    group.state = "connecting";
    group.generation += 1;
    const generation = group.generation;
    group.handshake = this.#handshakes.schedule(group.key.origin, (release) => {
      if (!this.#groupCurrent(group, generation, "connecting")) {
        release();
        return;
      }
      group.releaseHandshake = release;
      group.handshakeTimer = this.#timers.timeout(() => {
        group.handshakeTimer = null;
        this.#failed(group, generation, "transport_lost");
      }, this.#handshakeTimeoutMs);
      if (group.memberships.size === 0) {
        this.#releaseHandshake(group);
        return;
      }
      const request: DocumentTransportConnectRequest = Object.freeze({
        authorization: group.authorization,
        failed: (reason: DocumentTransportFailure) => {
          this.#failed(group, generation, reason);
        },
        key: group.key,
        message: (encoded: string) => {
          this.#message(group, generation, encoded);
        },
        opened: () => {
          this.#opened(group, generation);
        },
        transportGeneration: generation,
      });
      try {
        const port =
          group.key.transport === "sse"
            ? this.#transports.eventSource(request)
            : this.#transports.webSocket(request);
        if (!this.#groupCurrent(group, generation, "connecting")) {
          try {
            port.close("transport_replaced");
          } catch {
            // A synchronously failed/opened adapter is never retained.
          }
          return;
        }
        group.port = port;
      } catch {
        this.#failed(group, generation, "transport_lost");
      }
    });
  }

  #opened(group: PhysicalGroup, generation: number): void {
    if (!this.#groupCurrent(group, generation, "connecting")) return;
    this.#releaseHandshake(group);
    group.handshake = null;
    group.state = "open";
    for (const membership of group.memberships.values()) {
      this.#safeState(membership, "connecting");
      this.#subscribeMembership(group, membership, generation);
    }
  }

  #message(group: PhysicalGroup, generation: number, encoded: string): void {
    if (!this.#groupCurrent(group, generation, "open")) return;
    let subscriptionId: string;
    try {
      subscriptionId = inspectAsyncEnvelopeSubscription(encoded);
    } catch {
      this.#failed(group, generation, "protocol_invalid");
      return;
    }
    const membership = group.memberships.get(subscriptionId);
    if (membership === undefined || !membership.active || membership.group !== group) return;
    if (membership.authenticatedTransportGeneration !== generation) {
      if (
        membership.logicallyDegraded &&
        membership.quarantinedGroup === group &&
        membership.quarantinedTransportGeneration === generation &&
        membership.quarantinedDescriptorBinding === membership.authorization.descriptorBinding
      ) {
        return;
      }
      this.#failed(group, generation, "authorization_lost");
      return;
    }
    try {
      membership.sink.envelope(encoded);
    } catch {
      this.#safeState(membership, "degraded");
    }
  }

  #failed(group: PhysicalGroup, generation: number, reason: DocumentTransportFailure): void {
    if (
      !this.#groupCurrent(group, generation) ||
      (group.state !== "connecting" && group.state !== "open")
    ) {
      return;
    }
    // Backoff retires this physical callback before invoking adapter cleanup.
    // The next connect advances the generation exactly once, so its server-
    // authenticated generation is the immediate physical successor.
    group.state = "backoff";
    this.#releaseHandshake(group);
    const handshake = group.handshake;
    group.handshake = null;
    handshake?.cancel();
    for (const membership of group.memberships.values()) {
      this.#cancelMembershipAttachment(membership);
      if (membership.pendingAuthorizationKind !== "initial") {
        this.#discardPendingAuthorization(membership);
      }
    }
    const port = group.port;
    group.port = null;
    try {
      port?.close("transport_replaced");
    } catch {
      // A failed adapter cannot keep document ownership alive.
    }
    group.recoveryActive = true;
    group.recoveryGeneration += 1;
    for (const membership of group.memberships.values()) {
      membership.authenticatedTransportGeneration = -1;
      this.#clearMembershipQuarantine(membership);
      membership.pendingProofMembershipGeneration = -1;
      membership.pendingObservedTransportGeneration = -1;
      membership.recoveryRequestGeneration = -1;
      membership.provedTransportGeneration = -1;
      group.recovering.set(membership.authorization.subscriptionId, membership);
    }
    group.memberships.clear();
    for (const membership of group.recovering.values()) {
      membership.recoveryRequestGeneration = -1;
      this.#safeState(
        membership,
        reason === "authorization_lost" || reason === "protocol_invalid"
          ? "degraded"
          : "reconnecting",
      );
    }
    if (
      !this.#transportAvailable() ||
      reason === "authorization_lost" ||
      reason === "protocol_invalid"
    ) {
      if (reason === "authorization_lost" || reason === "protocol_invalid") {
        this.#retireConflictedGroup(group);
      }
      return;
    }
    if (group.recovering.size === 0) return;
    group.reconnectAttempt += 1;
    if (group.reconnectAttempt > group.policy.maximumAttempts) {
      this.#retireConflictedGroup(group);
      return;
    }
    const policy = group.policy;
    const exponent = Math.min(group.reconnectAttempt - 1, 30);
    const ceiling = Math.min(policy.maximumDelayMs, policy.minimumDelayMs * 2 ** exponent);
    const delay = Math.floor(validRandom(this.#randomness.number()) * (ceiling + 1));
    group.reconnectTimer = this.#timers.timeout(() => {
      group.reconnectTimer = null;
      if (this.#transportAvailable() && group.state === "backoff") {
        group.state = "restoring";
        const recoveryGeneration = group.recoveryGeneration;
        for (const membership of group.recovering.values()) {
          this.#restoreMembership(group, membership, recoveryGeneration);
        }
      }
    }, delay);
  }

  #remove(membership: LogicalMembership): void {
    if (!membership.active) return;
    membership.active = false;
    membership.generation += 1;
    this.#cancelMembershipAttachment(membership);
    this.#discardPendingAuthorization(membership);
    this.#clearMembershipQuarantine(membership);
    this.#memberships.delete(membership.authorization.subscriptionId);
    const group = membership.group;
    membership.group = null;
    this.#safeState(membership, "closed");
    if (group === null) return;
    const subscriptionId = membership.authorization.subscriptionId;
    const wasReady = group.memberships.delete(subscriptionId);
    group.recovering.delete(subscriptionId);
    if (group.recovering.size === 0) group.recoveryActive = false;
    if (group.memberships.size !== 0) this.#refreshGroupAuthority(group);
    if (group.state === "open" && wasReady) {
      try {
        group.port?.unsubscribe(subscriptionId);
      } catch {
        // Logical removal remains final even when adapter cleanup fails.
      }
    }
    if (group.memberships.size !== 0 || group.recovering.size !== 0) return;
    this.#closeGroup(group, "subscription_empty");
    this.#groups.delete(group.keyValue);
  }

  #closeGroup(group: PhysicalGroup, reason: DocumentTransportCloseReason): void {
    group.generation += 1;
    if (group.reconnectTimer !== null) {
      this.#timers.clearTimeout(group.reconnectTimer);
      group.reconnectTimer = null;
    }
    this.#releaseHandshake(group);
    group.handshake?.cancel();
    group.handshake = null;
    for (const membership of group.memberships.values()) {
      this.#cancelMembershipAttachment(membership);
      this.#discardPendingAuthorization(membership);
      this.#clearMembershipQuarantine(membership);
      membership.authenticatedTransportGeneration = -1;
      membership.pendingObservedTransportGeneration = -1;
    }
    for (const membership of group.recovering.values()) {
      this.#cancelMembershipAttachment(membership);
      this.#discardPendingAuthorization(membership);
      this.#clearMembershipQuarantine(membership);
      membership.authenticatedTransportGeneration = -1;
      membership.pendingObservedTransportGeneration = -1;
    }
    try {
      group.port?.close(reason);
    } catch {
      // Retirement still releases all local ownership.
    }
    group.port = null;
    group.state = "closed";
  }

  #restoreMembership(
    group: PhysicalGroup,
    membership: LogicalMembership,
    recoveryGeneration: number,
  ): void {
    if (
      !this.#transportAvailable() ||
      !group.recoveryActive ||
      group.recoveryGeneration !== recoveryGeneration ||
      group.recovering.get(membership.authorization.subscriptionId) !== membership ||
      membership.recoveryRequestGeneration === recoveryGeneration
    ) {
      return;
    }
    membership.recoveryRequestGeneration = recoveryGeneration;
    if (
      membership.pendingAuthorizationKind === "initial" &&
      membership.pendingAuthorization !== null
    ) {
      this.#admitRestoredMembership(
        group,
        membership,
        membership.pendingAuthorization,
        recoveryGeneration,
      );
      return;
    }
    const poolGeneration = this.#generation;
    void this.#requestReauthorization(membership, poolGeneration).then((current) => {
      if (!this.#recoveryCurrent(group, membership, recoveryGeneration)) return;
      const staged = current === null ? null : this.#acceptedReauthorization(membership, current);
      const accepted = staged?.subscription ?? null;
      if (accepted === null || staged === null || !sameKey(accepted.document, group.key)) {
        discardReauthorization(current);
        group.recovering.delete(membership.authorization.subscriptionId);
        membership.group = null;
        if (membership.active) this.#safeState(membership, "degraded");
        this.#finishRecovery(group);
        return;
      }
      membership.pendingAuthorization = staged;
      membership.pendingAuthorizationKind = "successor";
      this.#admitRestoredMembership(group, membership, staged, recoveryGeneration);
    });
  }

  #admitRestoredMembership(
    group: PhysicalGroup,
    membership: LogicalMembership,
    staged: ReauthorizedLogicalSubscription,
    recoveryGeneration: number,
  ): void {
    if (!this.#recoveryCurrent(group, membership, recoveryGeneration)) return;
    const accepted = staged.subscription;
    const subscriptions = [
      ...[...group.memberships.values()].map((candidate) =>
        this.#effectiveAuthorization(candidate),
      ),
      accepted,
    ];
    let authorization: AsyncTransportAuthorization;
    let policy: AsyncReconnectPolicy;
    try {
      authorization = commonAuthorization(subscriptions);
      policy = aggregatePolicy(subscriptions);
    } catch {
      if (membership.pendingAuthorizationKind !== "initial") {
        discardReauthorization(staged);
      }
      this.#retireConflictedGroup(group);
      return;
    }
    group.recovering.delete(membership.authorization.subscriptionId);
    membership.provedTransportGeneration = -1;
    group.memberships.set(accepted.subscriptionId, membership);
    group.authorization = authorization;
    group.policy = policy;
    this.#safeState(membership, "connecting");
    if (group.state === "restoring") this.#connect(group, "restoring");
    else if (group.state === "open") {
      this.#subscribeMembership(group, membership, group.generation);
    }
    this.#finishRecovery(group);
  }

  #recoveryCurrent(
    group: PhysicalGroup,
    membership: LogicalMembership,
    recoveryGeneration: number,
  ): boolean {
    return (
      this.#transportAvailable() &&
      this.#groups.get(group.keyValue) === group &&
      group.recoveryActive &&
      group.recoveryGeneration === recoveryGeneration &&
      group.state !== "backoff" &&
      group.state !== "closed" &&
      membership.active &&
      membership.group === group &&
      group.recovering.get(membership.authorization.subscriptionId) === membership
    );
  }

  #finishRecovery(group: PhysicalGroup): void {
    if (group.recovering.size !== 0) return;
    group.recoveryActive = false;
    this.#finishContinuity(group);
    if (group.memberships.size !== 0) return;
    this.#closeGroup(group, "subscription_empty");
    this.#groups.delete(group.keyValue);
  }

  #acceptedReauthorization(
    membership: LogicalMembership,
    result: ReauthorizedLogicalSubscription,
    initial = false,
  ): ReauthorizedLogicalSubscription | null {
    let current: unknown;
    let proof: unknown;
    try {
      current = Reflect.get(result, "subscription");
      proof = Reflect.get(result, "proof");
    } catch {
      return null;
    }
    if (
      (typeof current !== "object" && typeof current !== "function") ||
      current === null ||
      (proof !== "authoritative_no_tail" &&
        proof !== "complete_replay" &&
        !(initial && proof === null))
    ) {
      return null;
    }
    const authorization = current as AuthorizedLogicalSubscription;
    return membership.active &&
      authorization.subscriptionId === membership.authorization.subscriptionId &&
      authorization.stream === membership.authorization.stream &&
      typeof result.commit === "function" &&
      typeof result.discard === "function"
      ? result
      : null;
  }

  #requestReauthorization(
    membership: LogicalMembership,
    generation: number,
    initial = false,
  ): Promise<ReauthorizedLogicalSubscription | null> {
    let resolveResult!: ReauthorizationRequest["resolve"];
    const result = new Promise<ReauthorizedLogicalSubscription | null>((resolve) => {
      resolveResult = resolve;
    });
    const completion: ReauthorizationCompletion = { settle: null };
    const request: ReauthorizationRequest = {
      abort: null,
      completion,
      generation,
      membership,
      membershipGeneration: ++membership.generation,
      prior: initial ? null : membership.authorization,
      resolve: resolveResult,
      result,
      settled: false,
      timer: null,
    };
    completion.settle = (current) => {
      const activeMembership = request.membership;
      this.#settleReauthorization(
        request,
        activeMembership !== null &&
          activeMembership.active &&
          activeMembership.generation === request.membershipGeneration &&
          this.#generation === request.generation
          ? current
          : null,
      );
    };
    this.#reauthorizationQueue.push(request);
    this.#pumpReauthorizations();
    return result;
  }

  #pumpReauthorizations(): void {
    while (
      this.#activeReauthorizations.size < this.#reauthorizationConcurrency &&
      this.#reauthorizationQueue.length !== 0
    ) {
      const request = this.#reauthorizationQueue.shift();
      if (request === undefined || request.settled) continue;
      const membership = request.membership;
      const prior = request.prior;
      if (
        membership === null ||
        !membership.active ||
        membership.generation !== request.membershipGeneration ||
        this.#generation !== request.generation
      ) {
        this.#settleReauthorization(request, null);
        continue;
      }
      request.abort = new AbortController();
      this.#activeReauthorizations.add(request);
      const completion = request.completion;
      const signal = request.abort.signal;
      const operation = () => {
        if (!membership.active || request.settled) {
          this.#settleReauthorization(request, null);
          return request.result;
        }
        request.timer = this.#timers.timeout(() => {
          request.abort?.abort();
          this.#settleReauthorization(request, null);
        }, this.#reauthorizationTimeoutMs);
        let pending: ReauthorizedLogicalSubscription | Promise<ReauthorizedLogicalSubscription>;
        try {
          pending = membership.sink.reauthorize(prior, signal);
        } catch {
          this.#settleReauthorization(request, null);
          return request.result;
        }
        void Promise.resolve(pending).then(
          (current) => {
            completeReauthorization(completion, current);
          },
          () => {
            completeReauthorization(completion, null);
          },
        );
        return request.result;
      };
      const pending =
        this.#authorizationScheduler === null
          ? operation()
          : this.#authorizationScheduler.schedule(1, signal, operation);
      void pending.catch(() => {
        this.#settleReauthorization(request, null);
      });
    }
  }

  #degradeLogicalMembership(membership: LogicalMembership): void {
    const group = membership.group;
    if (
      !membership.active ||
      group?.state !== "open" ||
      group.memberships.get(membership.authorization.subscriptionId) !== membership
    ) {
      return;
    }
    const transportGeneration = group.generation;
    membership.generation += 1;
    this.#cancelMembershipAttachment(membership);
    this.#discardPendingAuthorization(membership);
    membership.quarantinedDescriptorBinding = membership.authorization.descriptorBinding;
    membership.quarantinedGroup = group;
    membership.quarantinedTransportGeneration = transportGeneration;
    membership.authenticatedTransportGeneration = -1;
    membership.pendingProofMembershipGeneration = -1;
    membership.pendingObservedTransportGeneration = -1;
    membership.provedTransportGeneration = -1;
    membership.logicallyDegraded = true;
    try {
      group.port?.unsubscribe(membership.authorization.subscriptionId);
    } catch {
      // Logical degradation remains exact when adapter cleanup fails.
    }
    this.#safeState(membership, "degraded");
    const poolGeneration = this.#generation;
    const pending = this.#requestReauthorization(membership, poolGeneration);
    const membershipGeneration = membership.generation;
    void pending.then((current) => {
      if (
        !membership.active ||
        membership.generation !== membershipGeneration ||
        membership.group !== group ||
        !this.#groupCurrent(group, transportGeneration, "open") ||
        group.memberships.get(membership.authorization.subscriptionId) !== membership
      ) {
        discardReauthorization(current);
        return;
      }
      const staged = current === null ? null : this.#acceptedReauthorization(membership, current);
      if (staged === null) {
        discardReauthorization(current);
        this.#safeState(membership, "degraded");
        return;
      }
      membership.pendingAuthorization = staged;
      membership.pendingAuthorizationKind = "successor";
      this.#safeState(membership, "connecting");
      this.#subscribeMembership(group, membership, transportGeneration);
    });
  }

  #settleReauthorization(
    request: ReauthorizationRequest,
    current: ReauthorizedLogicalSubscription | null,
  ): void {
    if (request.settled) return;
    request.settled = true;
    request.completion.settle = null;
    if (request.timer !== null) this.#timers.clearTimeout(request.timer);
    request.timer = null;
    request.abort = null;
    request.membership = null;
    request.prior = null;
    this.#activeReauthorizations.delete(request);
    request.resolve(current);
    this.#pumpReauthorizations();
  }

  #cancelReauthorizations(): void {
    const queued = this.#reauthorizationQueue.splice(0);
    for (const request of queued) this.#settleReauthorization(request, null);
    for (const request of [...this.#activeReauthorizations]) {
      request.abort?.abort();
      this.#settleReauthorization(request, null);
    }
  }

  #refreshGroupAuthority(group: PhysicalGroup): void {
    const subscriptions = [...group.memberships.values()].map((membership) =>
      this.#effectiveAuthorization(membership),
    );
    group.authorization = commonAuthorization(subscriptions);
    group.policy = aggregatePolicy(subscriptions);
  }

  #retireConflictedGroup(group: PhysicalGroup): void {
    this.#closeGroup(group, "transport_replaced");
    this.#groups.delete(group.keyValue);
    const memberships = new Set([...group.memberships.values(), ...group.recovering.values()]);
    for (const membership of memberships) {
      membership.group = null;
      this.#safeState(membership, "degraded");
    }
    group.memberships.clear();
    group.recovering.clear();
    group.recoveryActive = false;
  }

  #releaseHandshake(group: PhysicalGroup): void {
    if (group.handshakeTimer !== null) {
      this.#timers.clearTimeout(group.handshakeTimer);
      group.handshakeTimer = null;
    }
    const release = group.releaseHandshake;
    group.releaseHandshake = null;
    release?.();
  }

  #groupCurrent(group: PhysicalGroup, generation: number, state?: PhysicalGroup["state"]): boolean {
    return (
      this.#transportAvailable() &&
      this.#groups.get(group.keyValue) === group &&
      group.generation === generation &&
      (state === undefined || group.state === state)
    );
  }

  #transportAvailable(): boolean {
    return this.#state === "active" || this.#state === "resuming";
  }

  #safeState(membership: LogicalMembership, state: SubscriptionState): void {
    try {
      membership.sink.state(state);
    } catch {
      // Observer failure cannot rewrite transport or membership authority.
    }
  }

  #subscribeMembership(
    group: PhysicalGroup,
    membership: LogicalMembership,
    transportGeneration: number,
  ): void {
    this.#cancelMembershipAttachment(membership);
    const port = group.port;
    if (port === null) {
      this.#failed(group, transportGeneration, "authorization_lost");
      return;
    }
    const completion: MembershipAttachmentCompletion = { settle: null };
    completion.settle = (acknowledgment) => {
      this.#settleMembershipAttachment(
        completion,
        group,
        membership,
        transportGeneration,
        membership.generation,
        acknowledgment,
      );
    };
    membership.attachmentCompletion = completion;
    let pending: DocumentMembershipOutcome | Promise<DocumentMembershipOutcome>;
    try {
      pending = port.subscribe(this.#effectiveAuthorization(membership));
    } catch {
      completeMembershipAttachment(completion, null);
      return;
    }
    if (
      this.#isMembershipAcknowledgment(
        pending,
        this.#effectiveAuthorization(membership),
        transportGeneration,
      )
    ) {
      completeMembershipAttachment(completion, pending);
      return;
    }
    void Promise.resolve(pending).then(
      (acknowledgment) => {
        completeMembershipAttachment(completion, acknowledgment);
      },
      () => {
        completeMembershipAttachment(completion, null);
      },
    );
  }

  #settleMembershipAttachment(
    completion: MembershipAttachmentCompletion,
    group: PhysicalGroup,
    membership: LogicalMembership,
    transportGeneration: number,
    membershipGeneration: number,
    acknowledgment: unknown,
  ): void {
    if (completion.settle === null) return;
    completion.settle = null;
    if (membership.attachmentCompletion === completion) membership.attachmentCompletion = null;
    if (
      !this.#groupCurrent(group, transportGeneration, "open") ||
      !membership.active ||
      membership.group !== group ||
      membership.generation !== membershipGeneration ||
      group.memberships.get(membership.authorization.subscriptionId) !== membership
    ) {
      return;
    }
    if (
      !this.#isMembershipAcknowledgment(
        acknowledgment,
        this.#effectiveAuthorization(membership),
        transportGeneration,
      )
    ) {
      this.#failed(group, transportGeneration, "authorization_lost");
      return;
    }
    const staged = membership.pendingAuthorization;
    let presentationPending = false;
    if (staged !== null) {
      const stagedKind = membership.pendingAuthorizationKind;
      membership.pendingAuthorization = null;
      membership.pendingAuthorizationKind = null;
      const outcome: ReturnType<ReauthorizedLogicalSubscription["commit"]> = (() => {
        try {
          return staged.commit();
        } catch {
          return "stale";
        }
      })();
      if (outcome === "stale") {
        discardReauthorization(staged);
        this.#safeState(membership, "degraded");
        return;
      }
      presentationPending = outcome === "pending";
      membership.authorization = staged.subscription;
      if (stagedKind === "initial") membership.requiresInitialAuthorization = false;
      if (outcome === "committed" && staged.proof !== null) {
        membership.pendingProofMembershipGeneration = membership.generation;
      }
    }
    membership.authenticatedTransportGeneration = transportGeneration;
    this.#clearMembershipQuarantine(membership);
    if (
      staged !== null &&
      staged.proof !== null &&
      membership.pendingProofMembershipGeneration !== membership.generation
    ) {
      if (presentationPending) return;
      this.#safeState(membership, "degraded");
      return;
    }
    this.#consumeContinuityProof(group, membership, transportGeneration);
    if (membership.pendingObservedTransportGeneration === transportGeneration) {
      membership.pendingObservedTransportGeneration = -1;
      this.#proveContinuity(group, membership, transportGeneration);
    }
  }

  #cancelMembershipAttachment(membership: LogicalMembership): void {
    const completion = membership.attachmentCompletion;
    membership.attachmentCompletion = null;
    if (completion !== null) completion.settle = null;
  }

  #discardPendingAuthorization(membership: LogicalMembership): void {
    const staged = membership.pendingAuthorization;
    membership.pendingAuthorization = null;
    membership.pendingAuthorizationKind = null;
    if (staged === null) return;
    try {
      discardReauthorization(staged);
    } catch {
      // A dropped stage has no authority and cleanup is best-effort.
    }
  }

  #clearMembershipQuarantine(membership: LogicalMembership): void {
    membership.quarantinedDescriptorBinding = null;
    membership.quarantinedGroup = null;
    membership.quarantinedTransportGeneration = -1;
  }

  #effectiveAuthorization(membership: LogicalMembership): AuthorizedLogicalSubscription {
    return membership.pendingAuthorization?.subscription ?? membership.authorization;
  }

  #isMembershipAcknowledgment(
    acknowledgment: unknown,
    subscription: AuthorizedLogicalSubscription,
    transportGeneration: number,
  ): acknowledgment is DocumentMembershipAcknowledgment {
    if (
      (typeof acknowledgment !== "object" && typeof acknowledgment !== "function") ||
      acknowledgment === null
    ) {
      return false;
    }
    try {
      return (
        Reflect.get(acknowledgment, "kind") === "authenticated" &&
        Reflect.get(acknowledgment, "subscriptionId") === subscription.subscriptionId &&
        Reflect.get(acknowledgment, "descriptorBinding") === subscription.descriptorBinding &&
        Reflect.get(acknowledgment, "stream") === subscription.stream &&
        Reflect.get(acknowledgment, "transportGeneration") === transportGeneration
      );
    } catch {
      return false;
    }
  }

  #consumeContinuityProof(
    group: PhysicalGroup,
    membership: LogicalMembership,
    transportGeneration: number,
  ): void {
    if (membership.pendingProofMembershipGeneration !== membership.generation) return;
    membership.pendingProofMembershipGeneration = -1;
    this.#proveContinuity(group, membership, transportGeneration);
  }

  #proveContinuity(
    group: PhysicalGroup,
    membership: LogicalMembership,
    transportGeneration: number,
  ): void {
    if (membership.authenticatedTransportGeneration !== transportGeneration) {
      membership.pendingObservedTransportGeneration = transportGeneration;
      return;
    }
    if (
      !membership.active ||
      group.state !== "open" ||
      group.generation !== transportGeneration ||
      membership.group !== group ||
      group.memberships.get(membership.authorization.subscriptionId) !== membership
    ) {
      return;
    }
    membership.provedTransportGeneration = transportGeneration;
    if (membership.logicallyDegraded) {
      membership.logicallyDegraded = false;
      this.#safeState(membership, "current");
    }
    this.#finishContinuity(group);
  }

  #finishContinuity(group: PhysicalGroup): void {
    if (
      group.state !== "open" ||
      group.recoveryActive ||
      group.continuityTransportGeneration === group.generation ||
      ![...group.memberships.values()].every(
        (candidate) => candidate.provedTransportGeneration === group.generation,
      )
    ) {
      return;
    }
    group.continuityTransportGeneration = group.generation;
    group.reconnectAttempt = 0;
    for (const candidate of group.memberships.values()) this.#safeState(candidate, "current");
  }
}
