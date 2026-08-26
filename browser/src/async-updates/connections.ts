import { inspectAsyncEnvelopeSubscription } from "./envelope.js";
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
  reauthorize(
    prior: AuthorizedLogicalSubscription,
    signal: AbortSignal,
  ): ReauthorizedLogicalSubscription | Promise<ReauthorizedLogicalSubscription>;
  state(state: SubscriptionState): void;
}

export interface ReauthorizedLogicalSubscription {
  readonly proof: "authoritative_no_tail" | "complete_replay";
  readonly subscription: AuthorizedLogicalSubscription;
}

export interface LogicalSubscriptionHandle {
  close(): void;
  continuityProved(): void;
  heartbeatLost(): void;
}

export interface DocumentConnectionPoolOptions {
  readonly handshakeScheduler: OriginHandshakeScheduler;
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
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
    key: DocumentTransportKey,
    signal: AbortSignal,
  ) => Promise<void> | void;
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
  settled: boolean;
  timer: number | null;
}

interface QueuedMembershipControl {
  readonly operation: "subscribe" | "unsubscribe";
  readonly subscription: AuthorizedLogicalSubscription;
}

interface MembershipControlCompletion {
  settle: ((successful: boolean) => void) | null;
}

function completeMembershipControl(
  completion: MembershipControlCompletion,
  successful: boolean,
): void {
  completion.settle?.(successful);
}

class SseMembershipControls {
  readonly #membership: BrowserAsyncTransportOptions["sseMembership"];
  readonly #pending = new Set<PendingMembershipControl>();
  readonly #queue: QueuedMembershipControl[] = [];
  readonly #request: DocumentTransportConnectRequest;
  readonly #timeoutMs: number;
  readonly #timers: AsyncTimerPort;
  #closed = false;

  constructor(
    request: DocumentTransportConnectRequest,
    membership: BrowserAsyncTransportOptions["sseMembership"],
    timers: AsyncTimerPort,
    timeoutMs: number,
  ) {
    this.#request = request;
    this.#membership = membership;
    this.#timers = timers;
    this.#timeoutMs = timeoutMs;
  }

  request(
    operation: "subscribe" | "unsubscribe",
    subscription: AuthorizedLogicalSubscription,
  ): void {
    if (this.#closed) return;
    if (this.#pending.size + this.#queue.length >= MAX_QUEUED_SSE_MEMBERSHIP_CONTROLS) {
      this.#request.failed("authorization_lost");
      return;
    }
    this.#queue.push({ operation, subscription });
    this.#pump();
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#queue.splice(0);
    for (const control of [...this.#pending]) {
      control.abort?.abort();
      this.#settle(control, true);
    }
  }

  #start(request: QueuedMembershipControl): void {
    const completion: MembershipControlCompletion = { settle: null };
    const control: PendingMembershipControl = {
      abort: new AbortController(),
      completion,
      settled: false,
      timer: null,
    };
    completion.settle = (successful) => {
      this.#settle(control, successful);
    };
    this.#pending.add(control);
    control.timer = this.#timers.timeout(() => {
      control.abort?.abort();
      completeMembershipControl(completion, false);
    }, this.#timeoutMs);
    let pending: Promise<void> | void;
    try {
      const abort = control.abort;
      if (abort === null) return;
      pending = this.#membership(
        request.operation,
        request.subscription,
        this.#request.key,
        abort.signal,
      );
    } catch {
      completeMembershipControl(completion, false);
      return;
    }
    void Promise.resolve(pending).then(
      () => {
        completeMembershipControl(completion, true);
      },
      () => {
        completeMembershipControl(completion, false);
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

  #settle(control: PendingMembershipControl, successful: boolean): void {
    if (control.settled) return;
    control.settled = true;
    control.completion.settle = null;
    if (control.timer !== null) this.#timers.clearTimeout(control.timer);
    control.timer = null;
    control.abort = null;
    this.#pending.delete(control);
    if (!successful && !this.#closed) {
      this.#request.failed("authorization_lost");
      return;
    }
    this.#pump();
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
    this.#controls = new SseMembershipControls(request, membership, timers, membershipTimeoutMs);
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
    this.#controls.request("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    this.#controls.request("unsubscribe", subscription);
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
    this.#request = request;
    this.#controls = new SseMembershipControls(request, membership, timers, membershipTimeoutMs);
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
    this.#controls.request("subscribe", subscription);
  }

  unsubscribe(subscriptionId: string): void {
    if (this.#closed) return;
    const subscription = this.#subscriptions.get(subscriptionId);
    if (subscription === undefined) return;
    this.#subscriptions.delete(subscriptionId);
    this.#controls.request("unsubscribe", subscription);
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
      JSON.stringify({
        descriptor_binding: subscription.descriptorBinding,
        kind: "subscribe",
        stream: subscription.stream,
        subscription: subscription.subscriptionId,
      }),
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
  generation: number;
  pendingProofMembershipGeneration: number;
  provedTransportGeneration: number;
  recoveryRequestGeneration: number;
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
  releaseHandshake: VoidFunction | null;
  reconnectAttempt: number;
  reconnectTimer: number | null;
  state: "idle" | "backoff" | "restoring" | "connecting" | "open" | "closed";
}

interface ReauthorizationRequest {
  abort: AbortController | null;
  active: boolean;
  readonly completion: ReauthorizationCompletion;
  generation: number;
  membership: LogicalMembership | null;
  membershipGeneration: number;
  prior: AuthorizedLogicalSubscription | null;
  resolve(value: ReauthorizedLogicalSubscription | null): void;
  settled: boolean;
  timer: number | null;
}

interface ReauthorizationCompletion {
  settle: ((current: ReauthorizedLogicalSubscription | null) => void) | null;
}

function completeReauthorization(
  completion: ReauthorizationCompletion,
  current: ReauthorizedLogicalSubscription | null,
): void {
  completion.settle?.(current);
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
  readonly #handshakes: OriginHandshakeScheduler;
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
    if (!Number.isSafeInteger(concurrency) || concurrency < 1 || concurrency > 8) {
      throw new RangeError("async_reauthorization_concurrency_invalid");
    }
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30_000) {
      throw new RangeError("async_reauthorization_timeout_invalid");
    }
    this.#handshakes = options.handshakeScheduler;
    this.#randomness = options.randomness;
    this.#reauthorizationConcurrency = concurrency;
    this.#reauthorizationTimeoutMs = timeout;
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
      generation: 0,
      group: null,
      pendingProofMembershipGeneration: -1,
      provedTransportGeneration: -1,
      recoveryRequestGeneration: -1,
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
      this.#cancelReauthorizations();
      return;
    }
    this.#state = "suspended";
    this.#generation += 1;
    this.#cancelReauthorizations();
    for (const group of this.#groups.values()) this.#closeGroup(group, "page_suspended");
    this.#groups.clear();
    for (const membership of this.#memberships.values()) {
      membership.group = null;
      membership.pendingProofMembershipGeneration = -1;
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
        const current = await this.#requestReauthorization(membership, generation);
        if (!this.#resumeCurrent(generation)) return;
        const accepted =
          current === null ? null : this.#acceptedReauthorization(membership, current);
        if (accepted === null) {
          if (membership.active) this.#safeState(membership, "degraded");
          return;
        }
        membership.authorization = accepted;
        membership.pendingProofMembershipGeneration = membership.generation;
        try {
          this.#attach(membership);
        } catch {
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
      membership.group = null;
      this.#safeState(membership, "closed");
    }
    this.#memberships.clear();
  }

  #resumeCurrent(generation: number): boolean {
    return this.#state === "resuming" && this.#generation === generation;
  }

  #attach(membership: LogicalMembership): void {
    const key = normalizedKey(membership.authorization.document);
    const encodedKey = keyValue(key);
    let group = this.#groups.get(encodedKey);
    if (group === undefined) {
      group = {
        authorization: membership.authorization.authorization,
        continuityTransportGeneration: -1,
        generation: 0,
        handshake: null,
        key,
        keyValue: encodedKey,
        memberships: new Map(),
        policy: aggregatePolicy([membership.authorization]),
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
      const readySubscriptions = [...group.memberships.values()].map(
        ({ authorization }) => authorization,
      );
      const subscriptions = [
        ...readySubscriptions,
        ...[...group.recovering.values()].map(({ authorization }) => authorization),
        membership.authorization,
      ];
      try {
        if (group.recoveryActive && readySubscriptions.length === 0) {
          if (!sameAuthorization(group.authorization, membership.authorization.authorization)) {
            throw new Error("async_transport_authority_conflict");
          }
        } else {
          group.authorization = commonAuthorization([
            ...readySubscriptions,
            membership.authorization,
          ]);
        }
        group.policy = aggregatePolicy(subscriptions);
      } catch (error: unknown) {
        this.#retireConflictedGroup(group);
        throw error;
      }
    }
    if (group.recoveryActive) {
      group.recovering.set(membership.authorization.subscriptionId, membership);
      membership.group = group;
      this.#safeState(membership, "connecting");
      if (group.state !== "backoff") {
        this.#restoreMembership(group, membership, group.recoveryGeneration);
      }
      return;
    }
    group.memberships.set(membership.authorization.subscriptionId, membership);
    membership.group = group;
    this.#safeState(membership, "connecting");
    if (group.state === "open") {
      this.#safeSubscribe(group.port, membership.authorization);
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
      if (group.memberships.size === 0) {
        release();
        group.releaseHandshake = null;
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
    for (const membership of group.memberships.values()) {
      const subscribed = this.#safeSubscribe(group.port, membership.authorization);
      this.#safeState(membership, "connecting");
      if (subscribed) this.#consumeContinuityProof(group, membership, generation);
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
    group.state = "backoff";
    group.recoveryActive = true;
    group.recoveryGeneration += 1;
    for (const membership of group.memberships.values()) {
      membership.pendingProofMembershipGeneration = -1;
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
    const poolGeneration = this.#generation;
    void this.#requestReauthorization(membership, poolGeneration).then((current) => {
      if (!this.#recoveryCurrent(group, membership, recoveryGeneration)) return;
      const accepted = current === null ? null : this.#acceptedReauthorization(membership, current);
      if (accepted === null || !sameKey(accepted.document, group.key)) {
        group.recovering.delete(membership.authorization.subscriptionId);
        membership.group = null;
        if (membership.active) this.#safeState(membership, "degraded");
        this.#finishRecovery(group);
        return;
      }
      const subscriptions = [
        ...[...group.memberships.values()].map(({ authorization }) => authorization),
        accepted,
      ];
      let authorization: AsyncTransportAuthorization;
      let policy: AsyncReconnectPolicy;
      try {
        authorization = commonAuthorization(subscriptions);
        policy = aggregatePolicy(subscriptions);
      } catch {
        this.#retireConflictedGroup(group);
        return;
      }
      group.recovering.delete(membership.authorization.subscriptionId);
      membership.authorization = accepted;
      membership.pendingProofMembershipGeneration = membership.generation;
      membership.provedTransportGeneration = -1;
      group.memberships.set(accepted.subscriptionId, membership);
      group.authorization = authorization;
      group.policy = policy;
      this.#safeState(membership, "connecting");
      if (group.state === "restoring") this.#connect(group, "restoring");
      else if (group.state === "open") {
        if (this.#safeSubscribe(group.port, accepted)) {
          this.#consumeContinuityProof(group, membership, group.generation);
        }
      }
      this.#finishRecovery(group);
    });
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
  ): AuthorizedLogicalSubscription | null {
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
      (proof !== "authoritative_no_tail" && proof !== "complete_replay")
    ) {
      return null;
    }
    const authorization = current as AuthorizedLogicalSubscription;
    return membership.active &&
      authorization.subscriptionId === membership.authorization.subscriptionId &&
      authorization.stream === membership.authorization.stream
      ? authorization
      : null;
  }

  #requestReauthorization(
    membership: LogicalMembership,
    generation: number,
  ): Promise<ReauthorizedLogicalSubscription | null> {
    return new Promise((resolve) => {
      const completion: ReauthorizationCompletion = { settle: null };
      const request: ReauthorizationRequest = {
        abort: null,
        active: false,
        completion,
        generation,
        membership,
        membershipGeneration: ++membership.generation,
        prior: membership.authorization,
        resolve,
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
    });
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
        prior === null ||
        !membership.active ||
        membership.generation !== request.membershipGeneration ||
        this.#generation !== request.generation
      ) {
        this.#settleReauthorization(request, null);
        continue;
      }
      request.active = true;
      request.abort = new AbortController();
      this.#activeReauthorizations.add(request);
      request.timer = this.#timers.timeout(() => {
        request.abort?.abort();
        this.#settleReauthorization(request, null);
      }, this.#reauthorizationTimeoutMs);
      let pending: ReauthorizedLogicalSubscription | Promise<ReauthorizedLogicalSubscription>;
      try {
        pending = membership.sink.reauthorize(prior, request.abort.signal);
      } catch {
        this.#settleReauthorization(request, null);
        continue;
      }
      const completion = request.completion;
      void Promise.resolve(pending).then(
        (current) => {
          completeReauthorization(completion, current);
        },
        () => {
          completeReauthorization(completion, null);
        },
      );
    }
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
    const subscriptions = [...group.memberships.values()].map(({ authorization }) => authorization);
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

  #safeSubscribe(
    port: DocumentTransportPort | null,
    subscription: AuthorizedLogicalSubscription,
  ): boolean {
    try {
      if (port === null) return false;
      port.subscribe(subscription);
      return true;
    } catch {
      // Adapter membership failure is handled by its transport failure callback.
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
