import { inspectAsyncEnvelopeSubscription } from "./envelope.js";
import type {
  AsyncRandomness,
  AsyncTimerPort,
  AsyncTransportAuthorization,
  AuthorizedLogicalSubscription,
  DocumentTransportKey,
  SubscriptionState,
} from "./types.js";

const MAX_LOGICAL_SUBSCRIPTIONS = 256;
const MAX_PENDING_HANDSHAKES_PER_ORIGIN = 1_024;
const MAX_SSE_RECORD_BYTES = 65_536;
const ASYNC_EVENT_PATH = "/__live/async/events";

export type DocumentTransportCloseReason =
  "page_suspended" | "document_retired" | "transport_replaced" | "subscription_empty";

export type DocumentTransportFailure =
  "authorization_lost" | "heartbeat_lost" | "protocol_invalid" | "transport_lost";

export interface DocumentTransportConnectRequest {
  readonly authorization: AsyncTransportAuthorization;
  readonly key: DocumentTransportKey;
  failed(reason: DocumentTransportFailure): void;
  message(encoded: string): void;
  opened(): void;
}

export interface DocumentTransportPort {
  subscribe(subscription: AuthorizedLogicalSubscription): void;
  unsubscribe(subscriptionId: string): void;
  close(reason: DocumentTransportCloseReason): void;
}

export type EventSourcePort = DocumentTransportPort;
export type WebSocketPort = DocumentTransportPort;

export interface AsyncTransportPorts {
  eventSource(connect: DocumentTransportConnectRequest): EventSourcePort;
  webSocket(connect: DocumentTransportConnectRequest): WebSocketPort;
}

export interface LogicalSubscriptionSink {
  envelope(encoded: string): void;
  state(state: SubscriptionState): void;
}

export interface LogicalSubscriptionHandle {
  close(): void;
  heartbeatLost(): void;
}

export interface DocumentConnectionPoolOptions {
  readonly handshakeScheduler: OriginHandshakeScheduler;
  readonly randomness: AsyncRandomness;
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
  readonly sseMembership: (
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
    key: DocumentTransportKey,
    signal: AbortSignal,
  ) => Promise<void> | void;
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

class NativeEventSourceAdapter implements EventSourcePort {
  readonly #abort = new AbortController();
  readonly #native: NativeEventSourceLike;
  readonly #request: DocumentTransportConnectRequest;
  readonly #membership: BrowserAsyncTransportOptions["sseMembership"];
  readonly #subscriptions = new Map<string, AuthorizedLogicalSubscription>();
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    create: BrowserAsyncTransportOptions["eventSource"],
    membership: BrowserAsyncTransportOptions["sseMembership"],
  ) {
    this.#request = request;
    this.#membership = membership;
    const url = new URL(ASYNC_EVENT_PATH, request.key.origin).href;
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

  subscribe(subscription: AuthorizedLogicalSubscription): void {
    if (this.#closed) return;
    this.#subscriptions.set(subscription.subscriptionId, subscription);
    this.#control("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    this.#control("unsubscribe", subscription);
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#abort.abort();
    this.#subscriptions.clear();
    this.#native.close();
  }

  #control(
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
  ): void {
    let pending: Promise<void> | void;
    try {
      pending = this.#membership(operation, subscription, this.#request.key, this.#abort.signal);
    } catch {
      if (!this.#closed) this.#request.failed("authorization_lost");
      return;
    }
    void Promise.resolve(pending).catch(() => {
      if (!this.#closed) this.#request.failed("authorization_lost");
    });
  }
}

class FetchEventSourceAdapter implements EventSourcePort {
  readonly #abort = new AbortController();
  readonly #request: DocumentTransportConnectRequest;
  readonly #membership: BrowserAsyncTransportOptions["sseMembership"];
  readonly #subscriptions = new Map<string, AuthorizedLogicalSubscription>();
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    fetchPort: typeof globalThis.fetch,
    membership: BrowserAsyncTransportOptions["sseMembership"],
  ) {
    this.#request = request;
    this.#membership = membership;
    const authorization = request.authorization;
    if (authorization.kind !== "bearer" || authorization.credential.length === 0) {
      throw new Error("async_transport_authorization_invalid");
    }
    const url = new URL(ASYNC_EVENT_PATH, request.key.origin);
    const headers = new Headers({
      Accept: "text/event-stream",
      Authorization: `SuprnovaAsync ${authorization.credential}`,
    });
    void this.#read(fetchPort, url, headers);
  }

  subscribe(subscription: AuthorizedLogicalSubscription): void {
    if (this.#closed) return;
    this.#subscriptions.set(subscription.subscriptionId, subscription);
    this.#control("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    this.#control("unsubscribe", subscription);
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#subscriptions.clear();
    this.#abort.abort();
  }

  #control(
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
  ): void {
    let pending: Promise<void> | void;
    try {
      pending = this.#membership(operation, subscription, this.#request.key, this.#abort.signal);
    } catch {
      if (!this.#closed) this.#request.failed("authorization_lost");
      return;
    }
    void Promise.resolve(pending).catch(() => {
      if (!this.#closed) this.#request.failed("authorization_lost");
    });
  }

  async #read(fetchPort: typeof globalThis.fetch, url: URL, headers: Headers): Promise<void> {
    try {
      const response = await fetchPort(url.href, {
        cache: "no-store",
        credentials: "omit",
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
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    create: BrowserAsyncTransportOptions["webSocket"],
  ) {
    if (request.authorization.kind !== "session_cookie") {
      throw new Error("async_transport_authorization_invalid");
    }
    const url = new URL(ASYNC_EVENT_PATH, request.key.origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    this.#native = create(url.href);
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
    setHandler(this.#native, "onclose", () => {
      if (!this.#closed) request.failed("transport_lost");
    });
  }

  subscribe(subscription: AuthorizedLogicalSubscription): void {
    if (this.#closed) return;
    this.#native.send(
      JSON.stringify({ kind: "subscribe", subscription: subscription.subscriptionId }),
    );
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    this.#native.send(JSON.stringify({ kind: "unsubscribe", subscription: subscriptionId }));
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#native.close(1000, "suprnova_live_async_closed");
  }
}

export class BrowserAsyncTransportPorts implements AsyncTransportPorts {
  readonly #options: BrowserAsyncTransportOptions;

  constructor(options: BrowserAsyncTransportOptions) {
    if (
      typeof options.eventSource !== "function" ||
      typeof options.fetch !== "function" ||
      typeof options.sseMembership !== "function" ||
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
        )
      : new FetchEventSourceAdapter(connect, this.#options.fetch, this.#options.sseMembership);
  }

  webSocket(connect: DocumentTransportConnectRequest): WebSocketPort {
    return new BrowserWebSocketAdapter(connect, this.#options.webSocket);
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
  authorization: AuthorizedLogicalSubscription;
  readonly sink: LogicalSubscriptionSink;
  group: PhysicalGroup | null;
  active: boolean;
}

interface PhysicalGroup {
  readonly key: DocumentTransportKey;
  readonly keyValue: string;
  readonly memberships: Map<string, LogicalMembership>;
  generation: number;
  port: DocumentTransportPort | null;
  handshake: HandshakeRequest | null;
  releaseHandshake: VoidFunction | null;
  reconnectAttempt: number;
  reconnectTimer: number | null;
  state: "idle" | "connecting" | "open" | "closed";
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

export class DocumentConnectionPool {
  readonly #handshakes: OriginHandshakeScheduler;
  readonly #randomness: AsyncRandomness;
  readonly #timers: AsyncTimerPort;
  readonly #transports: AsyncTransportPorts;
  readonly #groups = new Map<string, PhysicalGroup>();
  readonly #memberships = new Map<string, LogicalMembership>();
  #generation = 0;
  #state: "active" | "suspended" | "retired" = "active";

  constructor(options: DocumentConnectionPoolOptions) {
    this.#handshakes = options.handshakeScheduler;
    this.#randomness = options.randomness;
    this.#timers = options.timers;
    this.#transports = options.transports;
  }

  subscribe(
    authorization: AuthorizedLogicalSubscription,
    sink: LogicalSubscriptionSink,
  ): LogicalSubscriptionHandle {
    if (this.#state === "retired") throw new Error("async_document_retired");
    if (this.#memberships.size >= MAX_LOGICAL_SUBSCRIPTIONS) {
      throw new Error("async_subscription_limit");
    }
    if (this.#memberships.has(authorization.subscriptionId)) {
      throw new Error("async_subscription_duplicate");
    }
    const membership: LogicalMembership = {
      active: true,
      authorization,
      group: null,
      sink,
    };
    this.#memberships.set(authorization.subscriptionId, membership);
    if (this.#state === "active") this.#attach(membership);
    return Object.freeze({
      close: () => {
        this.#remove(membership);
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
    });
  }

  suspend(): void {
    if (this.#state === "retired") return;
    if (this.#state === "suspended") {
      this.#generation += 1;
      return;
    }
    this.#state = "suspended";
    this.#generation += 1;
    for (const group of this.#groups.values()) this.#closeGroup(group, "page_suspended");
    this.#groups.clear();
    for (const membership of this.#memberships.values()) {
      membership.group = null;
      this.#safeState(membership, "disconnected");
    }
  }

  async resume(
    authorize: (
      prior: AuthorizedLogicalSubscription,
    ) => AuthorizedLogicalSubscription | Promise<AuthorizedLogicalSubscription>,
  ): Promise<void> {
    if (this.#state !== "suspended") return;
    const generation = ++this.#generation;
    const memberships = [...this.#memberships.values()].filter(({ active }) => active);
    const accepted: LogicalMembership[] = [];
    for (const membership of memberships) {
      let current: AuthorizedLogicalSubscription;
      try {
        current = await authorize(membership.authorization);
      } catch {
        if (this.#resumeCurrent(generation) && membership.active) {
          this.#safeState(membership, "degraded");
        }
        continue;
      }
      if (!this.#resumeCurrent(generation) || !membership.active) {
        return;
      }
      if (
        current.subscriptionId !== membership.authorization.subscriptionId ||
        current.stream !== membership.authorization.stream
      ) {
        this.#safeState(membership, "degraded");
        continue;
      }
      membership.authorization = current;
      accepted.push(membership);
    }
    if (!this.#resumeCurrent(generation)) return;
    this.#state = "active";
    for (const membership of accepted) this.#attach(membership);
  }

  dispose(): void {
    if (this.#state === "retired") return;
    this.#state = "retired";
    this.#generation += 1;
    for (const group of this.#groups.values()) this.#closeGroup(group, "document_retired");
    this.#groups.clear();
    for (const membership of this.#memberships.values()) {
      membership.active = false;
      membership.group = null;
      this.#safeState(membership, "closed");
    }
    this.#memberships.clear();
  }

  #resumeCurrent(generation: number): boolean {
    return this.#state === "suspended" && this.#generation === generation;
  }

  #attach(membership: LogicalMembership): void {
    const key = normalizedKey(membership.authorization.document);
    const encodedKey = keyValue(key);
    let group = this.#groups.get(encodedKey);
    if (group === undefined) {
      group = {
        generation: 0,
        handshake: null,
        key,
        keyValue: encodedKey,
        memberships: new Map(),
        port: null,
        reconnectAttempt: 0,
        reconnectTimer: null,
        releaseHandshake: null,
        state: "idle",
      };
      this.#groups.set(encodedKey, group);
    }
    group.memberships.set(membership.authorization.subscriptionId, membership);
    membership.group = group;
    this.#safeState(membership, "connecting");
    if (group.state === "open") {
      this.#safeSubscribe(group.port, membership.authorization);
    } else if (group.state === "idle") {
      this.#connect(group);
    }
  }

  #connect(group: PhysicalGroup): void {
    if (this.#state !== "active" || group.state !== "idle" || group.memberships.size === 0) return;
    group.state = "connecting";
    group.generation += 1;
    const generation = group.generation;
    group.handshake = this.#handshakes.schedule(group.key.origin, (release) => {
      if (!this.#groupCurrent(group, generation, "connecting")) {
        release();
        return;
      }
      group.releaseHandshake = release;
      const first = group.memberships.values().next().value;
      if (first === undefined) {
        release();
        group.releaseHandshake = null;
        return;
      }
      const request: DocumentTransportConnectRequest = Object.freeze({
        authorization: first.authorization.authorization,
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
      });
      try {
        group.port =
          group.key.transport === "sse"
            ? this.#transports.eventSource(request)
            : this.#transports.webSocket(request);
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
    group.reconnectAttempt = 0;
    for (const membership of group.memberships.values()) {
      this.#safeSubscribe(group.port, membership.authorization);
      this.#safeState(membership, "connecting");
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
    try {
      membership.sink.envelope(encoded);
    } catch {
      this.#safeState(membership, "degraded");
    }
  }

  #failed(group: PhysicalGroup, generation: number, reason: DocumentTransportFailure): void {
    if (!this.#groupCurrent(group, generation)) return;
    this.#releaseHandshake(group);
    group.handshake?.cancel();
    group.handshake = null;
    try {
      group.port?.close("transport_replaced");
    } catch {
      // A failed adapter cannot keep document ownership alive.
    }
    group.port = null;
    group.state = "idle";
    for (const membership of group.memberships.values()) {
      this.#safeState(
        membership,
        reason === "authorization_lost" || reason === "protocol_invalid"
          ? "degraded"
          : "reconnecting",
      );
    }
    if (
      this.#state !== "active" ||
      reason === "authorization_lost" ||
      reason === "protocol_invalid"
    ) {
      return;
    }
    const first = group.memberships.values().next().value;
    if (first === undefined) return;
    group.reconnectAttempt += 1;
    if (group.reconnectAttempt > first.authorization.reconnect.maximumAttempts) {
      for (const membership of group.memberships.values()) this.#safeState(membership, "degraded");
      return;
    }
    const policy = first.authorization.reconnect;
    const exponent = Math.min(group.reconnectAttempt - 1, 30);
    const ceiling = Math.min(policy.maximumDelayMs, policy.minimumDelayMs * 2 ** exponent);
    const delay = Math.floor(validRandom(this.#randomness.number()) * (ceiling + 1));
    group.reconnectTimer = this.#timers.timeout(() => {
      group.reconnectTimer = null;
      if (this.#state === "active" && group.state === "idle") this.#connect(group);
    }, delay);
  }

  #remove(membership: LogicalMembership): void {
    if (!membership.active) return;
    membership.active = false;
    this.#memberships.delete(membership.authorization.subscriptionId);
    const group = membership.group;
    membership.group = null;
    this.#safeState(membership, "closed");
    if (group === null) return;
    group.memberships.delete(membership.authorization.subscriptionId);
    if (group.state === "open") {
      try {
        group.port?.unsubscribe(membership.authorization.subscriptionId);
      } catch {
        // Logical removal remains final even when adapter cleanup fails.
      }
    }
    if (group.memberships.size !== 0) return;
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
    try {
      group.port?.close(reason);
    } catch {
      // Retirement still releases all local ownership.
    }
    group.port = null;
    group.state = "closed";
  }

  #releaseHandshake(group: PhysicalGroup): void {
    const release = group.releaseHandshake;
    group.releaseHandshake = null;
    release?.();
  }

  #groupCurrent(group: PhysicalGroup, generation: number, state?: PhysicalGroup["state"]): boolean {
    return (
      this.#state === "active" &&
      this.#groups.get(group.keyValue) === group &&
      group.generation === generation &&
      (state === undefined || group.state === state)
    );
  }

  #safeState(membership: LogicalMembership, state: SubscriptionState): void {
    try {
      membership.sink.state(state);
    } catch {
      // Observer failure cannot rewrite transport or membership authority.
    }
  }

  #safeSubscribe(
    port: DocumentTransportPort | null,
    subscription: AuthorizedLogicalSubscription,
  ): void {
    try {
      port?.subscribe(subscription);
    } catch {
      // Adapter membership failure is handled by its transport failure callback.
    }
  }
}
