import {
  defineAsyncFeature,
  type FeatureIslandController,
  type RuntimeFeature,
  type RuntimeFeatureDefinition,
  type RuntimeFeatureDocumentContext,
  type RuntimeFeatureIslandPort,
  type RegisteredBrowserEventCapability,
} from "../features/contract.js";
import { parseFeatureDirective } from "../features/directive-parser.js";
import {
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type AsyncTransportPorts,
  type DocumentAuthorizationScheduler,
  type DocumentAuthorizationSource,
  type LogicalSubscriptionHandle,
  type ReauthorizedLogicalSubscription,
} from "./connections.js";
import { AsyncSubscription } from "./subscription.js";
import type {
  AsyncClock,
  AsyncDispatchPort,
  AsyncPayload,
  AsyncRandomness,
  AsyncTimerPort,
  AuthorizedLogicalSubscription,
  StreamPosition,
  SubscriptionState,
} from "./types.js";

const MAX_STREAMS_PER_ISLAND = 1;
const MAX_U64 = (1n << 64n) - 1n;
const MAX_AUTHORIZATION_TEXT = 1_024;
const MAX_EVENTS = 64;
const MAX_EVENT_TARGETS = 16;
const MAX_PRESENTATION_SIGNALS = 64;
const MAX_RECONNECT_ATTEMPTS = 16;
const MAX_TRANSPORT_DELAY_MS = 300_000;
const MAX_CONCURRENT_AUTHORIZATIONS = 8;
const AUTHORIZATION_TIMEOUT_MS = 5_000;
const OPERATION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const PAYLOAD_CONTRACT = /^[a-z][a-z0-9._/-]{0,127}$/u;
const SUBSCRIPTION_ID = /^[A-Za-z0-9_-]{16,128}$/u;

export interface AsyncAuthorizationRequest {
  readonly identity: RuntimeFeatureIslandPort["identity"];
  readonly position: StreamPosition | null;
  readonly prior: AuthorizedLogicalSubscription | null;
  readonly signal: AbortSignal;
  readonly stream: string;
}

export interface AsyncAuthorityPort {
  authorize(
    request: AsyncAuthorizationRequest,
  ):
    | AsyncAuthorizationResult
    | AuthorizedLogicalSubscription
    | Promise<AsyncAuthorizationResult | AuthorizedLogicalSubscription>;
}

export interface AsyncAuthorizationResult {
  readonly replay: readonly string[];
  readonly subscription: AuthorizedLogicalSubscription;
}

interface AuthorityCompletion {
  settle: ((outcome: AuthorityOutcome) => void) | null;
}

type AuthorityOutcome =
  | Readonly<{
      kind: "authorized";
      value: AsyncAuthorizationResult | AuthorizedLogicalSubscription;
    }>
  | Readonly<{ kind: "rejected"; reason: string }>;

function completeAuthority(completion: AuthorityCompletion, outcome: AuthorityOutcome): void {
  completion.settle?.(outcome);
}

function invokeAuthority(
  authority: AsyncAuthorityPort,
  timers: AsyncTimerPort,
  abort: AbortController,
  request: AsyncAuthorizationRequest,
): Promise<AsyncAuthorizationResult | AuthorizedLogicalSubscription> {
  return new Promise((resolve, reject) => {
    const completion: AuthorityCompletion = { settle: null };
    let timer: number | null = null;
    const settle = (outcome: AuthorityOutcome) => {
      if (completion.settle === null) return;
      completion.settle = null;
      abort.signal.removeEventListener("abort", aborted);
      if (timer !== null) timers.clearTimeout(timer);
      timer = null;
      if (outcome.kind === "authorized") resolve(outcome.value);
      else reject(new Error(outcome.reason));
    };
    const aborted = () => {
      completeAuthority(
        completion,
        Object.freeze({ kind: "rejected", reason: "async_authorization_aborted" }),
      );
    };
    completion.settle = settle;
    abort.signal.addEventListener("abort", aborted, { once: true });
    timer = timers.timeout(() => {
      completeAuthority(
        completion,
        Object.freeze({ kind: "rejected", reason: "async_authorization_timeout" }),
      );
      abort.abort();
    }, AUTHORIZATION_TIMEOUT_MS);
    if (abort.signal.aborted) {
      aborted();
      return;
    }
    let pending: ReturnType<AsyncAuthorityPort["authorize"]>;
    try {
      pending = authority.authorize(request);
    } catch {
      completeAuthority(
        completion,
        Object.freeze({ kind: "rejected", reason: "async_authorization_rejected" }),
      );
      return;
    }
    void Promise.resolve(pending).then(
      (value) => {
        completeAuthority(completion, Object.freeze({ kind: "authorized", value }));
      },
      () => {
        completeAuthority(
          completion,
          Object.freeze({ kind: "rejected", reason: "async_authorization_rejected" }),
        );
      },
    );
  });
}

class AuthorizationInvocationScheduler implements DocumentAuthorizationScheduler {
  #active = 0;
  #next: DocumentAuthorizationSource = 0;
  #paused = false;
  readonly #queues: [VoidFunction[], VoidFunction[]] = [[], []];

  pause(): void {
    this.#paused = true;
  }

  resume(): void {
    if (!this.#paused) return;
    this.#paused = false;
    this.#pump();
  }

  schedule<T>(
    source: DocumentAuthorizationSource,
    signal: AbortSignal,
    operation: () => Promise<T>,
  ): Promise<T> {
    if (
      !this.#paused &&
      !signal.aborted &&
      this.#active < MAX_CONCURRENT_AUTHORIZATIONS &&
      this.#queues[0].length === 0 &&
      this.#queues[1].length === 0
    ) {
      this.#next = source === 0 ? 1 : 0;
      return this.#run(operation);
    }
    return new Promise((resolve, reject) => {
      let queued: typeof operation | null = operation;
      let queuedSignal: AbortSignal | null = signal;
      const abort = () => {
        queued = null;
        queuedSignal?.removeEventListener("abort", abort);
        queuedSignal = null;
        reject(new Error("async_authorization_aborted"));
      };
      const start = () => {
        const current = queued;
        queued = null;
        queuedSignal?.removeEventListener("abort", abort);
        const aborted = queuedSignal?.aborted === true;
        queuedSignal = null;
        if (current === null) return;
        if (aborted) {
          reject(new Error("async_authorization_aborted"));
          return;
        }
        void this.#run(current).then(resolve, reject);
      };
      signal.addEventListener("abort", abort, { once: true });
      this.#queues[source].push(start);
      this.#pump();
    });
  }

  #pump(): void {
    while (!this.#paused && this.#active < MAX_CONCURRENT_AUTHORIZATIONS) {
      const preferred = this.#queues[this.#next];
      const source = preferred.length === 0 ? (this.#next === 0 ? 1 : 0) : this.#next;
      const start = this.#queues[source].shift();
      if (start === undefined) return;
      this.#next = source === 0 ? 1 : 0;
      start();
    }
  }

  #run<T>(operation: () => Promise<T>): Promise<T> {
    this.#active += 1;
    let pending: Promise<T>;
    try {
      pending = operation();
    } catch (error: unknown) {
      this.#release();
      return Promise.reject(
        error instanceof Error ? error : new Error("async_authorization_rejected"),
      );
    }
    void pending.then(
      () => {
        this.#release();
      },
      () => {
        this.#release();
      },
    );
    return pending;
  }

  #release(): void {
    this.#active -= 1;
    this.#pump();
  }
}

export interface AsyncFeatureOptions {
  readonly authority: AsyncAuthorityPort;
  readonly clock: AsyncClock;
  readonly handshakeScheduler?: OriginHandshakeScheduler;
  readonly randomness: AsyncRandomness;
  readonly timers: AsyncTimerPort;
  readonly transports: AsyncTransportPorts;
}

function report(
  context: RuntimeFeatureDocumentContext,
  detail: "operation_rejected" | "resource_exhausted",
): void {
  try {
    context.diagnose(detail);
  } catch {
    // Diagnostics are bounded, redaction-safe, and best-effort.
  }
}

function validOptions(options: AsyncFeatureOptions): boolean {
  try {
    return (
      typeof options.authority.authorize === "function" &&
      typeof options.clock.now === "function" &&
      typeof options.randomness.number === "function" &&
      typeof options.timers.clearTimeout === "function" &&
      typeof options.timers.timeout === "function" &&
      typeof options.transports.eventSource === "function" &&
      typeof options.transports.webSocket === "function"
    );
  } catch {
    return false;
  }
}

function validPosition(position: StreamPosition): boolean {
  return (
    typeof position.epoch === "bigint" &&
    position.epoch >= 0n &&
    position.epoch <= MAX_U64 &&
    typeof position.sequence === "bigint" &&
    position.sequence >= 0n &&
    position.sequence <= MAX_U64
  );
}

function isAuthorizationResult(
  value: AsyncAuthorizationResult | AuthorizedLogicalSubscription,
): value is AsyncAuthorizationResult {
  try {
    return (
      Object.prototype.hasOwnProperty.call(value, "subscription") &&
      Array.isArray(Reflect.get(value, "replay"))
    );
  } catch {
    return false;
  }
}

function validateAuthorization(value: AuthorizedLogicalSubscription): void {
  let valid: boolean;
  try {
    const authorization = value.authorization;
    const authorizationValid =
      authorization.kind === "session_cookie" ||
      (authorization.credential.length >= 16 &&
        authorization.credential.length <= MAX_AUTHORIZATION_TEXT);
    const reconnect = value.reconnect;
    const eventNames = new Set(value.events.map(({ name }) => name));
    const signalNames = new Set(value.presentationSignals.map(({ name }) => name));
    valid =
      authorizationValid &&
      validPosition(value.baseline) &&
      value.descriptorBinding.length >= 1 &&
      value.descriptorBinding.length <= MAX_AUTHORIZATION_TEXT &&
      value.document.authorizationScope.length >= 1 &&
      value.document.authorizationScope.length <= 256 &&
      value.events.length <= MAX_EVENTS &&
      eventNames.size === value.events.length &&
      value.events.every((event) => {
        const cycle = event.cycle;
        const cycleKind: unknown = Reflect.get(cycle, "kind");
        const maximumHops: unknown = Reflect.get(cycle, "maximumHops");
        const order: unknown = Reflect.get(event, "order");
        const source: unknown = Reflect.get(event, "source");
        return (
          OPERATION_NAME.test(event.name) &&
          PAYLOAD_CONTRACT.test(event.payloadContract) &&
          source === "stream" &&
          order === "per_source_sequence" &&
          Number.isSafeInteger(event.version) &&
          event.version >= 1 &&
          event.version <= 65_535 &&
          Number.isSafeInteger(event.maximumFanout) &&
          event.maximumFanout >= event.targets.length &&
          event.maximumFanout <= 256 &&
          event.targets.length >= 1 &&
          event.targets.length <= MAX_EVENT_TARGETS &&
          new Set(event.targets).size === event.targets.length &&
          (cycleKind === "forbid_repeated_island" ||
            (cycleKind === "maximum_hops" &&
              Number.isSafeInteger(maximumHops) &&
              typeof maximumHops === "number" &&
              maximumHops >= 1 &&
              maximumHops <= 255))
        );
      }) &&
      Number.isSafeInteger(value.expiresAt) &&
      value.expiresAt >= 0 &&
      Number.isSafeInteger(value.heartbeatTimeoutMs) &&
      value.heartbeatTimeoutMs >= 1 &&
      value.heartbeatTimeoutMs <= MAX_TRANSPORT_DELAY_MS &&
      value.presentationSignals.length <= MAX_PRESENTATION_SIGNALS &&
      signalNames.size === value.presentationSignals.length &&
      value.presentationSignals.every(({ name }) => OPERATION_NAME.test(name)) &&
      Number.isSafeInteger(reconnect.maximumAttempts) &&
      reconnect.maximumAttempts >= 1 &&
      reconnect.maximumAttempts <= MAX_RECONNECT_ATTEMPTS &&
      Number.isSafeInteger(reconnect.minimumDelayMs) &&
      reconnect.minimumDelayMs >= 1 &&
      Number.isSafeInteger(reconnect.maximumDelayMs) &&
      reconnect.maximumDelayMs >= reconnect.minimumDelayMs &&
      reconnect.maximumDelayMs <= MAX_TRANSPORT_DELAY_MS &&
      OPERATION_NAME.test(value.stream) &&
      SUBSCRIPTION_ID.test(value.subscriptionId);
  } catch {
    valid = false;
  }
  if (!valid) throw new Error("async_authorization_invalid");
}

class AsyncIslandController implements FeatureIslandController {
  readonly #authorizationScheduler: AuthorizationInvocationScheduler;
  readonly #authority: AsyncAuthorityPort;
  readonly #clock: AsyncClock;
  readonly #context: RuntimeFeatureDocumentContext;
  readonly #owner: AsyncDocumentOwner;
  readonly #port: RuntimeFeatureIslandPort;
  readonly #stream: string;
  readonly #timers: AsyncTimerPort;
  #authorizationAbort: AbortController | null = null;
  #generation = 0;
  #heartbeatTimer: number | null = null;
  #handle: LogicalSubscriptionHandle | null = null;
  #currentAuthorization: AuthorizedLogicalSubscription | null = null;
  #pendingAuthorization: {
    readonly authorization: AuthorizedLogicalSubscription;
    readonly token: object;
  } | null = null;
  #eventCapability: RegisteredBrowserEventCapability | null = null;
  #state: "active" | "disposed" | "resuming" | "suspended" = "active";
  #subscription: AsyncSubscription | null = null;

  constructor(
    owner: AsyncDocumentOwner,
    context: RuntimeFeatureDocumentContext,
    port: RuntimeFeatureIslandPort,
    stream: string,
    options: AsyncFeatureOptions,
    authorizationScheduler: AuthorizationInvocationScheduler,
  ) {
    this.#authorizationScheduler = authorizationScheduler;
    this.#authority = options.authority;
    this.#clock = options.clock;
    this.#context = context;
    this.#owner = owner;
    this.#port = port;
    this.#stream = stream;
    this.#timers = options.timers;
  }

  start(): void {
    void this.#authorize(null).catch(() => {
      if (this.#state === "active") report(this.#context, "operation_rejected");
    });
  }

  authorizationId(): string | null {
    return this.#authorization()?.subscriptionId ?? null;
  }

  needsInitialResume(): boolean {
    return (
      this.#state === "suspended" &&
      this.#handle === null &&
      this.#subscription === null &&
      this.#authorization() === null
    );
  }

  async resumeInitial(): Promise<void> {
    if (!this.needsInitialResume()) return;
    this.#state = "resuming";
    try {
      await this.#authorize(null);
      if (this.#resuming()) {
        this.#state = "active";
        this.#armHeartbeat(this.#heartbeatTimeout());
      }
    } catch (error: unknown) {
      if (this.#state === "resuming") {
        this.#state = "suspended";
        report(this.#context, "operation_rejected");
      }
      throw error;
    }
  }

  async reauthorize(
    prior: AuthorizedLogicalSubscription | null,
    signal: AbortSignal,
  ): Promise<ReauthorizedLogicalSubscription> {
    const initial = prior === null;
    if (
      (this.#state !== "suspended" && this.#state !== "active") ||
      (initial ? this.#authorization() !== null : this.authorizationId() !== prior.subscriptionId)
    ) {
      throw new Error("async_reauthorization_invalid");
    }
    const resume = this.#state === "suspended";
    if (resume) this.#state = "resuming";
    try {
      const current = await this.#authorize(prior, signal, true);
      let settled = false;
      return Object.freeze({
        commit: () => {
          if (settled) return "stale";
          settled = true;
          const outcome = current.commit();
          if (outcome !== "stale" && resume && this.#resuming()) {
            this.#state = "active";
            this.#armHeartbeat(this.#heartbeatTimeout());
          } else if (resume && this.#state === "resuming") this.#state = "suspended";
          return outcome;
        },
        discard: () => {
          if (settled) return;
          settled = true;
          current.discard();
          if (resume && this.#state === "resuming") this.#state = "suspended";
        },
        proof: current.proof,
        subscription: current.subscription,
      });
    } catch (error: unknown) {
      if (resume && this.#state === "resuming") this.#state = "suspended";
      throw error;
    }
  }

  suspend(): void {
    if (this.#state !== "active" && this.#state !== "resuming") return;
    this.#state = "suspended";
    this.#generation += 1;
    this.#authorizationAbort?.abort();
    this.#authorizationAbort = null;
    this.#clearHeartbeat();
    this.#subscription?.transportLost();
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#generation += 1;
    this.#authorizationAbort?.abort();
    this.#authorizationAbort = null;
    this.#clearHeartbeat();
    this.#handle?.close();
    this.#handle = null;
    this.#subscription?.close();
    this.#subscription = null;
    this.#owner.retire(this);
  }

  async #authorize(
    prior: AuthorizedLogicalSubscription | null,
    externalSignal?: AbortSignal,
    admitted = false,
  ): Promise<ReauthorizedLogicalSubscription> {
    const generation = ++this.#generation;
    this.#authorizationAbort?.abort();
    const abort = new AbortController();
    this.#authorizationAbort = abort;
    const externallyAbort = () => {
      abort.abort();
    };
    externalSignal?.addEventListener("abort", externallyAbort, { once: true });
    if (externalSignal?.aborted === true) abort.abort();
    const position = prior === null ? null : (this.#subscription?.position() ?? null);
    let resolved: AsyncAuthorizationResult | AuthorizedLogicalSubscription;
    try {
      const request = Object.freeze({
        identity: this.#port.identity,
        position,
        prior,
        signal: abort.signal,
        stream: this.#stream,
      });
      const operation = () => invokeAuthority(this.#authority, this.#timers, abort, request);
      resolved = admitted
        ? await operation()
        : await this.#authorizationScheduler.schedule(0, abort.signal, operation);
    } finally {
      externalSignal?.removeEventListener("abort", externallyAbort);
      if (this.#authorizationAbort === abort) this.#authorizationAbort = null;
    }
    let current: AuthorizedLogicalSubscription;
    let replay: readonly string[];
    let carriesContinuityEvidence = false;
    if (isAuthorizationResult(resolved)) {
      carriesContinuityEvidence = true;
      current = resolved.subscription;
      replay = resolved.replay;
    } else {
      if (prior !== null) throw new Error("async_continuity_proof_required");
      current = resolved;
      replay = Object.freeze([]);
    }
    validateAuthorization(current);
    if (
      abort.signal.aborted ||
      this.#state === "disposed" ||
      this.#generation !== generation ||
      current.stream !== this.#stream
    ) {
      throw new Error("async_authorization_stale");
    }
    if (prior === null) {
      const subscription = this.#subscription;
      if (subscription === null) {
        return this.#install(current, replay, generation, carriesContinuityEvidence);
      }
      const proof = carriesContinuityEvidence
        ? subscription.preflightFreshInitialReplay(current, replay)
        : null;
      return this.#stageAuthorization(current, replay, proof, generation, true, true);
    }
    const subscription = this.#subscription;
    if (subscription === null || position === null) throw new Error("async_subscription_retired");
    const proof = subscription.preflightReauthorization(current, replay);
    return this.#stageAuthorization(current, replay, proof, generation);
  }

  #authorization(): AuthorizedLogicalSubscription | null {
    return this.#owner.authorization(this);
  }

  #resuming(): boolean {
    return this.#state === "resuming";
  }

  #install(
    authorization: AuthorizedLogicalSubscription,
    replay: readonly string[],
    generation: number,
    carriesContinuityEvidence: boolean,
  ): ReauthorizedLogicalSubscription {
    if (this.#subscription !== null || this.#handle !== null) {
      throw new Error("async_subscription_duplicate");
    }
    const dispatch: AsyncDispatchPort = Object.freeze({
      browserEvent: (event: Extract<AsyncPayload, { kind: "browser_event" }>) => {
        const contract = this.#currentAuthorization?.events.find(
          ({ name }) => name === event.event,
        );
        const capability = this.#eventCapability;
        return (
          contract !== undefined &&
          capability !== null &&
          this.#port.dispatchRegisteredEvent(capability, {
            event: event.event,
            payload: event.payload,
            schemaVersion: event.schema_version,
            target: event.target,
          }) === "dispatched"
        );
      },
      presentationSignal: (signal: Extract<AsyncPayload, { kind: "presentation_signal" }>) => {
        try {
          this.#port.writePresentationSignal(this.#port.element, signal.name, signal.value);
          return true;
        } catch {
          return false;
        }
      },
      refresh: () => this.#port.enqueueFreshRender("stream") !== "retired",
    });
    const subscription = new AsyncSubscription(authorization, dispatch, this.#clock);
    this.#subscription = subscription;
    const preflight = subscription.preflightInitialReplay(replay);
    const proof = carriesContinuityEvidence ? preflight : null;
    const staged = this.#stageAuthorization(authorization, replay, proof, generation, true);
    try {
      this.#handle = this.#owner.subscribe(
        authorization,
        {
          envelope: (encoded) => {
            try {
              const disposition = subscription.receive(encoded);
              if (
                (disposition === "applied" || disposition === "duplicate") &&
                subscription.state() === "current"
              ) {
                this.#handle?.continuityProved();
              } else if (disposition === "gap" || disposition === "continuity_required") {
                this.#handle?.continuityLost();
              }
              this.#armHeartbeat(this.#heartbeatTimeout());
            } catch {
              subscription.authorizationUncertain();
              report(this.#context, "operation_rejected");
            }
          },
          reauthorize: (prior, signal) => this.reauthorize(prior, signal),
          state: (state) => {
            this.#transportState(state);
          },
        },
        staged,
      );
    } catch (error: unknown) {
      staged.discard();
      this.#subscription = null;
      throw error;
    }
    return staged;
  }

  #stageAuthorization(
    authorization: AuthorizedLogicalSubscription,
    replay: readonly string[],
    proof: ReauthorizedLogicalSubscription["proof"],
    generation: number,
    initial = false,
    freshInitial = false,
  ): ReauthorizedLogicalSubscription {
    const subscription = this.#subscription;
    if (subscription === null) throw new Error("async_subscription_retired");
    let settled = false;
    const token = Object.freeze({});
    this.#pendingAuthorization = Object.freeze({ authorization, token });
    const clearPending = () => {
      if (this.#pendingAuthorization?.token === token) this.#pendingAuthorization = null;
    };
    return Object.freeze({
      commit: () => {
        if (
          settled ||
          this.#state === "disposed" ||
          this.#generation !== generation ||
          this.#subscription !== subscription
        ) {
          clearPending();
          return "stale";
        }
        settled = true;
        let capability: RegisteredBrowserEventCapability;
        let installed = false;
        try {
          capability = this.#port.authorizeRegisteredEvents(
            Object.freeze({
              descriptorBinding: authorization.descriptorBinding,
              events: authorization.events,
            }),
          );
          if (freshInitial) subscription.replaceUncommittedInitial(authorization);
          else if (!initial) subscription.reauthorize(authorization);
          this.#eventCapability = capability;
          this.#currentAuthorization = authorization;
          this.#owner.remember(this, authorization);
          installed = true;
          if (replay.length === 0) {
            if (proof !== null) subscription.proveAuthoritativeBaseline(authorization.baseline);
          } else {
            subscription.receiveReplay(replay);
          }
          clearPending();
          return "committed";
        } catch {
          clearPending();
          subscription.authorizationUncertain();
          report(this.#context, "operation_rejected");
          return installed ? "degraded" : "stale";
        }
      },
      discard: () => {
        settled = true;
        clearPending();
      },
      proof,
      subscription: authorization,
    });
  }

  #transportState(state: SubscriptionState): void {
    const subscription = this.#subscription;
    if (subscription === null || this.#state === "disposed") return;
    switch (state) {
      case "connecting":
        subscription.connected();
        this.#armHeartbeat(this.#heartbeatTimeout());
        break;
      case "reconnecting":
      case "disconnected":
        this.#clearHeartbeat();
        subscription.transportLost();
        break;
      case "degraded":
        this.#clearHeartbeat();
        subscription.authorizationUncertain();
        break;
      case "closed":
        this.#clearHeartbeat();
        subscription.close();
        break;
      case "current":
        this.#armHeartbeat(this.#heartbeatTimeout());
        break;
    }
  }

  #armHeartbeat(milliseconds: number): void {
    if (this.#state !== "active") return;
    this.#clearHeartbeat();
    this.#heartbeatTimer = this.#timers.timeout(() => {
      this.#heartbeatTimer = null;
      if (this.#state !== "active") return;
      this.#subscription?.heartbeatLost();
      this.#handle?.heartbeatLost();
    }, milliseconds);
  }

  #clearHeartbeat(): void {
    if (this.#heartbeatTimer === null) return;
    this.#timers.clearTimeout(this.#heartbeatTimer);
    this.#heartbeatTimer = null;
  }

  #heartbeatTimeout(): number {
    return (
      this.#pendingAuthorization?.authorization.heartbeatTimeoutMs ??
      this.#currentAuthorization?.heartbeatTimeoutMs ??
      1
    );
  }
}

export class AsyncDocumentOwner {
  readonly #authorizationScheduler = new AuthorizationInvocationScheduler();
  readonly #context: RuntimeFeatureDocumentContext;
  readonly #controllers = new Set<AsyncIslandController>();
  readonly #authorizations = new Map<AsyncIslandController, AuthorizedLogicalSubscription>();
  readonly #options: AsyncFeatureOptions;
  readonly #pool: DocumentConnectionPool;
  #state: "active" | "disposed" | "resuming" | "suspended" = "active";

  constructor(context: RuntimeFeatureDocumentContext, options: AsyncFeatureOptions) {
    if (!validOptions(options)) throw new Error("async_feature_configuration_invalid");
    this.#context = context;
    this.#options = options;
    this.#pool = new DocumentConnectionPool({
      authorizationScheduler: this.#authorizationScheduler,
      handshakeScheduler: options.handshakeScheduler ?? new OriginHandshakeScheduler(),
      randomness: options.randomness,
      timers: options.timers,
      transports: options.transports,
    });
  }

  connectIsland(port: RuntimeFeatureIslandPort): FeatureIslandController {
    if (this.#state === "disposed") throw new Error("async_document_retired");
    const streams = port
      .queryDirectiveOwnership(parseFeatureDirective)
      .filter(({ directive }) => directive.name === "stream" && directive.role === null);
    if (streams.length === 0) return Object.freeze({ dispose: () => undefined });
    if (streams.length > MAX_STREAMS_PER_ISLAND) {
      report(this.#context, "resource_exhausted");
      return Object.freeze({ dispose: () => undefined });
    }
    const controller = new AsyncIslandController(
      this,
      this.#context,
      port,
      streams[0]?.directive.value ?? "",
      this.#options,
      this.#authorizationScheduler,
    );
    this.#controllers.add(controller);
    controller.start();
    return controller;
  }

  subscribe(
    authorization: AuthorizedLogicalSubscription,
    sink: Parameters<DocumentConnectionPool["subscribe"]>[1],
    pendingAuthorization: ReauthorizedLogicalSubscription | null = null,
  ): LogicalSubscriptionHandle {
    return this.#pool.subscribe(authorization, sink, pendingAuthorization);
  }

  remember(controller: AsyncIslandController, authorization: AuthorizedLogicalSubscription): void {
    this.#authorizations.set(controller, authorization);
  }

  authorization(controller: AsyncIslandController): AuthorizedLogicalSubscription | null {
    return this.#authorizations.get(controller) ?? null;
  }

  retire(controller: AsyncIslandController): void {
    this.#controllers.delete(controller);
    this.#authorizations.delete(controller);
  }

  suspend(): void {
    if (this.#state !== "active" && this.#state !== "resuming") return;
    this.#state = "suspended";
    this.#authorizationScheduler.pause();
    for (const controller of this.#controllers) controller.suspend();
    this.#pool.suspend();
  }

  async resume(): Promise<void> {
    if (this.#state !== "suspended") return;
    this.#state = "resuming";
    this.#authorizationScheduler.resume();
    const initial = [...this.#controllers]
      .filter((controller) => controller.needsInitialResume())
      .map(async (controller) => {
        try {
          await controller.resumeInitial();
        } catch {
          // The controller already reports the typed authorization failure.
        }
      });
    await Promise.all([this.#pool.resume(), ...initial]);
    if (this.#resuming()) this.#state = "active";
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#authorizationScheduler.pause();
    for (const controller of [...this.#controllers]) controller.dispose();
    this.#pool.dispose();
  }

  #resuming(): boolean {
    return this.#state === "resuming";
  }
}

function configuredFeature(options: () => AsyncFeatureOptions | null): RuntimeFeature {
  const sharedHandshakes = new OriginHandshakeScheduler();
  const definition: RuntimeFeatureDefinition = Object.freeze({
    connectDocument(context: RuntimeFeatureDocumentContext) {
      const configured = options();
      if (configured === null) {
        return Object.freeze({
          connectIsland(port: RuntimeFeatureIslandPort) {
            if (
              port
                .queryDirectiveOwnership(parseFeatureDirective)
                .some(({ directive }) => directive.name === "stream")
            ) {
              report(context, "operation_rejected");
            }
            return undefined;
          },
          dispose() {
            // No resources exist before the application supplies async authority and transport ports.
          },
        });
      }
      const owner = new AsyncDocumentOwner(context, {
        ...configured,
        handshakeScheduler: configured.handshakeScheduler ?? sharedHandshakes,
      });
      return Object.freeze({
        connectIsland(port: RuntimeFeatureIslandPort) {
          return owner.connectIsland(port);
        },
        dispose() {
          owner.dispose();
        },
        resume() {
          void owner.resume().catch(() => {
            report(context, "operation_rejected");
          });
        },
        suspend() {
          owner.suspend();
        },
      });
    },
  });
  return defineAsyncFeature(definition);
}

export function createAsyncFeature(options: AsyncFeatureOptions): RuntimeFeature {
  if (!validOptions(options)) throw new Error("async_feature_configuration_invalid");
  return configuredFeature(() => options);
}

let defaultConfiguration: AsyncFeatureOptions | null = null;
let defaultConfigurationLocked = false;

export function configureAsync(options: AsyncFeatureOptions): void {
  if (defaultConfigurationLocked) throw new Error("async_configuration_locked");
  if (!validOptions(options)) throw new Error("async_feature_configuration_invalid");
  defaultConfiguration = Object.freeze({ ...options });
  defaultConfigurationLocked = true;
}

export const asyncFeature: RuntimeFeature = configuredFeature(() => {
  defaultConfigurationLocked = true;
  return defaultConfiguration;
});
