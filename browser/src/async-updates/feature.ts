import {
  defineAsyncFeature,
  type AsyncRuntimeFeatureDefinition,
  type AsyncRuntimeIslandPort,
  type FeatureIslandController,
  type RuntimeFeature,
  type RuntimeFeatureDocumentContext,
  type RegisteredBrowserEventCapability,
  sealValidatedAsyncDescriptor,
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
import { AsyncDispatcher } from "./dispatch.js";
import { AsyncSubscription } from "./subscription.js";
import {
  PollTimer,
  resolvePollPolicy,
  type PollEnvironment,
  type PollPolicy,
  type PollStatus,
} from "./poll.js";
import type {
  AsyncClock,
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
const SIGNAL_SCOPE = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
const SUBSCRIPTION_ID = /^[A-Za-z0-9_-]{16,128}$/u;

export interface AsyncAuthorizationRequest {
  readonly identity: AsyncRuntimeIslandPort["identity"];
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
  readonly authority?: AsyncAuthorityPort;
  readonly clock: AsyncClock;
  readonly handshakeScheduler?: OriginHandshakeScheduler;
  readonly observeFreshness?: AsyncFreshnessObserver;
  readonly pollEnvironment?: PollEnvironment;
  readonly randomness: AsyncRandomness;
  readonly timers: AsyncTimerPort;
  readonly transports?: AsyncTransportPorts;
}

export interface AsyncFreshnessObservation {
  readonly component: string;
  readonly documentKey: string;
  readonly slot: string;
  readonly state: PollStatus;
}

export type AsyncFreshnessObserver = (observation: AsyncFreshnessObservation) => void;

interface AsyncStreamFeatureOptions extends AsyncFeatureOptions {
  readonly authority: AsyncAuthorityPort;
  readonly transports: AsyncTransportPorts;
}

function globalEventTarget(name: "document" | "window"): EventTarget | null {
  const value: unknown = Reflect.get(globalThis, name);
  return value instanceof EventTarget ? value : null;
}

const browserPollEnvironment: PollEnvironment = Object.freeze({
  isOnline: () => {
    const navigator: unknown = Reflect.get(globalThis, "navigator");
    return (
      typeof navigator !== "object" ||
      navigator === null ||
      Reflect.get(navigator, "onLine") !== false
    );
  },
  isVisible: () => {
    const document: unknown = Reflect.get(globalThis, "document");
    return (
      typeof document !== "object" ||
      document === null ||
      Reflect.get(document, "visibilityState") !== "hidden"
    );
  },
  subscribe(listener: VoidFunction): VoidFunction {
    const window = globalEventTarget("window");
    const document = globalEventTarget("document");
    window?.addEventListener("online", listener);
    window?.addEventListener("offline", listener);
    document?.addEventListener("visibilitychange", listener);
    return () => {
      window?.removeEventListener("online", listener);
      window?.removeEventListener("offline", listener);
      document?.removeEventListener("visibilitychange", listener);
    };
  },
});

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

function samePollPolicy(left: PollPolicy | null, right: PollPolicy | null): boolean {
  return (
    left === right ||
    (left !== null &&
      right !== null &&
      left.initial === right.initial &&
      left.intervalMs === right.intervalMs &&
      left.jitterRatio === right.jitterRatio &&
      left.mode === right.mode &&
      left.visibility === right.visibility)
  );
}

function validOptions(options: AsyncFeatureOptions): boolean {
  try {
    const streamPortsAbsent = options.authority === undefined && options.transports === undefined;
    const streamPortsPresent =
      typeof options.authority?.authorize === "function" &&
      typeof options.transports?.eventSource === "function" &&
      typeof options.transports.webSocket === "function";
    const pollEnvironment = options.pollEnvironment ?? browserPollEnvironment;
    return (
      (streamPortsAbsent || streamPortsPresent) &&
      typeof options.clock.now === "function" &&
      (options.observeFreshness === undefined || typeof options.observeFreshness === "function") &&
      typeof pollEnvironment.isOnline === "function" &&
      typeof pollEnvironment.isVisible === "function" &&
      typeof pollEnvironment.subscribe === "function" &&
      typeof options.randomness.number === "function" &&
      typeof options.timers.clearTimeout === "function" &&
      typeof options.timers.timeout === "function"
    );
  } catch {
    return false;
  }
}

function streamOptions(options: AsyncFeatureOptions): AsyncStreamFeatureOptions | null {
  if (options.authority === undefined || options.transports === undefined) return null;
  return options as AsyncStreamFeatureOptions;
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
    const fallbackInitial: unknown = Reflect.get(value.fallbackPoll, "initial");
    const fallbackVisibility: unknown = Reflect.get(value.fallbackPoll, "visibility");
    const eventNames = new Set(value.events.map(({ name }) => name));
    const signalNames = new Set(
      value.presentationSignals.map(({ name, scope }) => `${scope}\0${name}`),
    );
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
      Number.isSafeInteger(value.fallbackPoll.intervalMs) &&
      value.fallbackPoll.intervalMs >= 1_000 &&
      value.fallbackPoll.intervalMs <= MAX_TRANSPORT_DELAY_MS &&
      Number.isFinite(value.fallbackPoll.jitterRatio) &&
      value.fallbackPoll.jitterRatio >= 0 &&
      value.fallbackPoll.jitterRatio <= 1 &&
      (fallbackInitial === "wait" || fallbackInitial === "immediate") &&
      (fallbackVisibility === "visible" || fallbackVisibility === "always") &&
      value.presentationSignals.length <= MAX_PRESENTATION_SIGNALS &&
      signalNames.size === value.presentationSignals.length &&
      value.presentationSignals.every(
        ({ name, schema, scope }) =>
          OPERATION_NAME.test(name) &&
          SIGNAL_SCOPE.test(scope) &&
          validPresentationSignalSchema(schema),
      ) &&
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

function validPresentationSignalSchema(value: unknown): boolean {
  return (
    value === "null" ||
    value === "boolean" ||
    value === "i64" ||
    value === "u64" ||
    value === "string"
  );
}

class AsyncIslandController implements FeatureIslandController {
  readonly #authorizationScheduler: AuthorizationInvocationScheduler;
  readonly #authority: AsyncAuthorityPort;
  readonly #clock: AsyncClock;
  readonly #context: RuntimeFeatureDocumentContext;
  readonly #freshnessObserver: ((state: PollStatus) => void) | undefined;
  readonly #owner: AsyncDocumentOwner;
  readonly #pollEnvironment: PollEnvironment;
  #pollModifiers: readonly string[] | null;
  readonly #port: AsyncRuntimeIslandPort;
  readonly #randomness: AsyncRandomness;
  readonly #stream: string;
  #streamModifiers: readonly string[];
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
  #poll: PollTimer | null = null;
  #pollPolicy: PollPolicy | null = null;
  #state: "active" | "disposed" | "resuming" | "suspended" = "active";
  #subscription: AsyncSubscription | null = null;

  constructor(
    owner: AsyncDocumentOwner,
    context: RuntimeFeatureDocumentContext,
    port: AsyncRuntimeIslandPort,
    stream: string,
    pollModifiers: readonly string[] | null,
    streamModifiers: readonly string[],
    options: AsyncStreamFeatureOptions,
    authorizationScheduler: AuthorizationInvocationScheduler,
  ) {
    this.#authorizationScheduler = authorizationScheduler;
    this.#authority = options.authority;
    this.#clock = options.clock;
    this.#context = context;
    this.#freshnessObserver = freshnessObserver(options, port);
    this.#owner = owner;
    this.#pollEnvironment = options.pollEnvironment ?? browserPollEnvironment;
    this.#pollModifiers = pollModifiers;
    this.#port = port;
    this.#randomness = options.randomness;
    this.#stream = stream;
    this.#streamModifiers = streamModifiers;
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

  configuredStream(): string {
    return this.#stream;
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
        this.#poll?.resume();
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
            this.#poll?.resume();
            this.#armHeartbeat(this.#heartbeatTimeout());
          } else if (resume) this.#resumeCommittedFallback();
          return outcome;
        },
        discard: () => {
          if (settled) return;
          settled = true;
          current.discard();
          if (resume) this.#resumeCommittedFallback();
        },
        proof: current.proof,
        subscription: current.subscription,
      });
    } catch (error: unknown) {
      if (resume) this.#resumeCommittedFallback();
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
    this.#poll?.suspend();
    this.#subscription?.transportLost();
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#generation += 1;
    this.#authorizationAbort?.abort();
    this.#authorizationAbort = null;
    this.#clearHeartbeat();
    this.#retirePoll();
    this.#handle?.close();
    this.#handle = null;
    this.#subscription?.close();
    this.#subscription = null;
    this.#owner.retire(this);
  }

  reconcileFreshness(
    pollModifiers: readonly string[] | null,
    streamModifiers: readonly string[],
  ): void {
    if (this.#state === "disposed") return;
    this.#pollModifiers = pollModifiers;
    this.#streamModifiers = streamModifiers;
    if (this.#pendingAuthorization !== null) {
      try {
        this.#resolvePollPolicy(this.#pendingAuthorization.authorization);
        const committedPolicy =
          this.#currentAuthorization === null
            ? null
            : this.#resolvePollPolicy(this.#currentAuthorization);
        if (!samePollPolicy(committedPolicy, this.#pollPolicy)) this.#retirePoll();
      } catch {
        this.#retirePoll();
        report(this.#context, "operation_rejected");
      }
      return;
    }
    const authorization = this.#currentAuthorization;
    if (authorization === null) return;
    const continuity = this.#subscription?.state() === "current" ? "current" : "degraded";
    try {
      this.#commitPollPolicy(this.#resolvePollPolicy(authorization), continuity, true);
    } catch {
      this.#retirePoll();
      report(this.#context, "operation_rejected");
    }
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
    const dispatcher = new AsyncDispatcher(this.#port, () => this.#eventCapability);
    const subscription = new AsyncSubscription(
      authorization,
      dispatcher,
      this.#clock,
      (completion) => {
        if (this.#state === "disposed" || this.#subscription !== subscription) return;
        if (completion === "succeeded" && subscription.state() === "current") {
          this.#handle?.continuityProved();
          this.#poll?.continuity("current");
        }
      },
      (outcome) => {
        if (this.#state === "disposed" || this.#subscription !== subscription) return;
        this.#clearHeartbeat();
        if (outcome.kind === "complete") {
          this.#retirePoll();
          const handle = this.#handle;
          this.#handle = null;
          handle?.close();
          return;
        }
        this.#handle?.presentationFailed();
        this.#poll?.continuity("degraded");
        report(this.#context, "operation_rejected");
      },
    );
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
                this.#poll?.continuity("current");
              } else if (disposition === "gap" || disposition === "continuity_required") {
                this.#handle?.continuityLost();
                this.#poll?.continuity("degraded");
              }
              if (
                subscription.state() === "closed" ||
                disposition === "dispatch_failed" ||
                (disposition === "applied" && subscription.state() === "degraded")
              ) {
                this.#clearHeartbeat();
              } else {
                this.#armHeartbeat(this.#heartbeatTimeout());
              }
            } catch {
              subscription.authorizationUncertain();
              this.#poll?.continuity("degraded");
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
    const clearPending = (): boolean => {
      if (this.#pendingAuthorization?.token !== token) return false;
      this.#pendingAuthorization = null;
      return true;
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
          const pollPolicy = this.#resolvePollPolicy(authorization);
          capability = this.#port.consumeRegisteredEventCapability(
            sealValidatedAsyncDescriptor(this.#port, authorization),
          );
          if (freshInitial) subscription.replaceUncommittedInitial(authorization);
          else if (!initial) subscription.reauthorize(authorization);
          this.#eventCapability = capability;
          this.#currentAuthorization = authorization;
          installed = true;
          let replayPending = false;
          if (replay.length === 0) {
            if (proof !== null) subscription.proveAuthoritativeBaseline(authorization.baseline);
          } else {
            replayPending = subscription.receiveReplay(replay) === "pending";
          }
          const continuity = subscription.state() === "current" ? "current" : "degraded";
          this.#owner.remember(this, authorization);
          this.#commitPollPolicy(pollPolicy, continuity);
          clearPending();
          return replayPending ? "pending" : "committed";
        } catch {
          clearPending();
          subscription.authorizationUncertain();
          this.#poll?.continuity("degraded");
          report(this.#context, "operation_rejected");
          return installed ? "degraded" : "stale";
        }
      },
      discard: () => {
        if (settled) return;
        settled = true;
        if (clearPending()) this.#restoreCommittedPollPolicy();
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
        this.#poll?.continuity("degraded");
        break;
      case "degraded":
        this.#clearHeartbeat();
        subscription.authorizationUncertain();
        this.#poll?.continuity("degraded");
        break;
      case "closed":
        this.#clearHeartbeat();
        subscription.close();
        this.#poll?.continuity("degraded");
        break;
      case "current":
        this.#armHeartbeat(this.#heartbeatTimeout());
        if (subscription.state() === "current") this.#poll?.continuity("current");
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
      this.#poll?.continuity("degraded");
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

  #resolvePollPolicy(authorization: AuthorizedLogicalSubscription): PollPolicy | null {
    return resolvePollPolicy(
      this.#pollModifiers,
      this.#streamModifiers,
      authorization.fallbackPoll,
    );
  }

  #commitPollPolicy(
    policy: PollPolicy | null,
    continuity: "current" | "degraded",
    applyInitial = false,
  ): void {
    if (policy === null) {
      this.#retirePoll();
      return;
    }
    if (this.#poll === null) {
      this.#poll = new PollTimer({
        enqueueFreshRender: (reason, completion, completionKey) =>
          this.#port.enqueueFreshRender(reason, completion, completionKey),
        environment: this.#pollEnvironment,
        observe: this.#freshnessObserver,
        policy,
        randomness: this.#randomness,
        timers: this.#timers,
      });
      this.#poll.continuity(continuity);
      if (this.#state === "suspended" || this.#state === "resuming") this.#poll.suspend();
      this.#poll.start();
    } else {
      this.#poll.updatePolicy(policy, applyInitial);
      this.#poll.continuity(continuity);
    }
    this.#pollPolicy = policy;
  }

  #retirePoll(): void {
    this.#poll?.dispose();
    this.#poll = null;
    this.#pollPolicy = null;
  }

  #restoreCommittedPollPolicy(): void {
    const authorization = this.#currentAuthorization;
    if (authorization === null || this.#state === "disposed") return;
    try {
      this.#commitPollPolicy(this.#resolvePollPolicy(authorization), "degraded");
    } catch {
      this.#retirePoll();
    }
  }

  #resumeCommittedFallback(): void {
    if (!this.#resuming() || this.#currentAuthorization === null) return;
    this.#state = "active";
    this.#restoreCommittedPollPolicy();
    this.#poll?.resume();
  }
}

class PollingIslandController implements FeatureIslandController {
  readonly #owner: AsyncDocumentOwner;
  readonly #timer: PollTimer;
  #disposed = false;

  constructor(
    owner: AsyncDocumentOwner,
    port: AsyncRuntimeIslandPort,
    policy: PollPolicy,
    options: AsyncFeatureOptions,
  ) {
    this.#owner = owner;
    this.#timer = new PollTimer({
      enqueueFreshRender: (reason, completion, completionKey) =>
        port.enqueueFreshRender(reason, completion, completionKey),
      environment: options.pollEnvironment ?? browserPollEnvironment,
      observe: freshnessObserver(options, port),
      policy,
      randomness: options.randomness,
      timers: options.timers,
    });
  }

  start(): void {
    this.#timer.start();
  }

  suspend(): void {
    this.#timer.suspend();
  }

  resume(): void {
    this.#timer.resume();
  }

  reconcileFreshness(policy: PollPolicy): void {
    this.#timer.updatePolicy(policy, true);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#timer.dispose();
    this.#owner.retirePoll(this);
  }
}

type ActiveAsyncIslandController = AsyncIslandController | PollingIslandController;

type AsyncIslandDirectiveState =
  | Readonly<{ kind: "none" }>
  | Readonly<{
      diagnostic: "operation_rejected" | "resource_exhausted";
      kind: "invalid";
    }>
  | Readonly<{ kind: "poll"; policy: PollPolicy }>
  | Readonly<{
      kind: "stream";
      pollModifiers: readonly string[] | null;
      stream: string;
      streamModifiers: readonly string[];
    }>;

function directiveState(port: AsyncRuntimeIslandPort): AsyncIslandDirectiveState {
  const ownership = port.queryDirectiveOwnership(parseFeatureDirective);
  const streams = ownership.filter(
    ({ directive }) => directive.name === "stream" && directive.role === null,
  );
  const polls = ownership.filter(
    ({ directive }) => directive.name === "poll" && directive.role === null,
  );
  if (streams.length > MAX_STREAMS_PER_ISLAND || polls.length > 1) {
    return {
      diagnostic: "resource_exhausted" as const,
      kind: "invalid" as const,
    };
  }
  const stream = streams[0]?.directive;
  const poll = polls[0]?.directive;
  if (stream === undefined) {
    try {
      const policy = resolvePollPolicy(poll?.modifiers ?? null, null, null);
      return policy === null
        ? { kind: "none" as const }
        : {
            kind: "poll" as const,
            policy,
          };
    } catch {
      return {
        diagnostic: "operation_rejected" as const,
        kind: "invalid" as const,
      };
    }
  }
  return {
    kind: "stream" as const,
    pollModifiers: poll?.modifiers ?? null,
    stream: stream.value,
    streamModifiers: stream.modifiers,
  };
}

function createAsyncIslandLifecycleController(
  owner: AsyncDocumentOwner,
  context: RuntimeFeatureDocumentContext,
  port: AsyncRuntimeIslandPort,
): FeatureIslandController {
  let active: ActiveAsyncIslandController | null = null;
  let disposed = false;
  let morphPending = false;

  const reconcile = (): void => {
    const next = directiveState(port);
    if (next.kind === "invalid") {
      report(context, next.diagnostic);
    }
    if (next.kind === "invalid" || next.kind === "none") {
      active?.dispose();
      active = null;
      return;
    }
    if (next.kind === "poll") {
      if (active instanceof PollingIslandController) {
        active.reconcileFreshness(next.policy);
        return;
      }
      active?.dispose();
      active = owner.activatePoll(port, next.policy);
      return;
    }
    if (active instanceof AsyncIslandController && active.configuredStream() === next.stream) {
      active.reconcileFreshness(next.pollModifiers, next.streamModifiers);
      return;
    }
    active?.dispose();
    active = owner.activateStream(port, next.stream, next.pollModifiers, next.streamModifiers);
  };

  reconcile();
  return {
    abortMorph(): void {
      morphPending = false;
    },
    afterMorph(): void {
      if (disposed || !morphPending) return;
      morphPending = false;
      reconcile();
    },
    beforeMorph(): void {
      if (!disposed) morphPending = true;
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      morphPending = false;
      active?.dispose();
      active = null;
    },
    resume(): void {
      if (active instanceof PollingIslandController) active.resume();
    },
    suspend(): void {
      active?.suspend();
    },
  };
}

function freshnessObserver(
  options: AsyncFeatureOptions,
  port: AsyncRuntimeIslandPort,
): ((state: PollStatus) => void) | undefined {
  const observer = options.observeFreshness;
  if (observer === undefined) return undefined;
  const { component, documentKey, slot } = port.identity;
  return (state) => {
    observer(Object.freeze({ component, documentKey, slot, state }));
  };
}

export class AsyncDocumentOwner {
  readonly #authorizationScheduler = new AuthorizationInvocationScheduler();
  readonly #context: RuntimeFeatureDocumentContext;
  readonly #controllers = new Set<AsyncIslandController>();
  readonly #pollControllers = new Set<PollingIslandController>();
  readonly #authorizations = new Map<AsyncIslandController, AuthorizedLogicalSubscription>();
  readonly #options: AsyncFeatureOptions;
  readonly #pool: DocumentConnectionPool | null;
  #state: "active" | "disposed" | "resuming" | "suspended" = "active";

  constructor(context: RuntimeFeatureDocumentContext, options: AsyncFeatureOptions) {
    if (!validOptions(options)) throw new Error("async_feature_configuration_invalid");
    this.#context = context;
    this.#options = options;
    const stream = streamOptions(options);
    this.#pool =
      stream === null
        ? null
        : new DocumentConnectionPool({
            authorizationScheduler: this.#authorizationScheduler,
            handshakeScheduler: options.handshakeScheduler ?? new OriginHandshakeScheduler(),
            randomness: options.randomness,
            timers: options.timers,
            transports: stream.transports,
          });
  }

  connectIsland(port: AsyncRuntimeIslandPort): FeatureIslandController {
    if (this.#state === "disposed") throw new Error("async_document_retired");
    return createAsyncIslandLifecycleController(this, this.#context, port);
  }

  activatePoll(port: AsyncRuntimeIslandPort, policy: PollPolicy): PollingIslandController {
    if (this.#state === "disposed") throw new Error("async_document_retired");
    const controller = new PollingIslandController(this, port, policy, this.#options);
    this.#pollControllers.add(controller);
    controller.start();
    if (this.#state === "suspended" || this.#state === "resuming") controller.suspend();
    return controller;
  }

  activateStream(
    port: AsyncRuntimeIslandPort,
    stream: string,
    pollModifiers: readonly string[] | null,
    streamModifiers: readonly string[],
  ): AsyncIslandController | null {
    if (this.#state === "disposed") throw new Error("async_document_retired");
    const configured = streamOptions(this.#options);
    if (configured === null || this.#pool === null) {
      report(this.#context, "operation_rejected");
      return null;
    }
    const controller = new AsyncIslandController(
      this,
      this.#context,
      port,
      stream,
      pollModifiers,
      streamModifiers,
      configured,
      this.#authorizationScheduler,
    );
    this.#controllers.add(controller);
    if (this.#state === "suspended" || this.#state === "resuming") controller.suspend();
    else controller.start();
    return controller;
  }

  subscribe(
    authorization: AuthorizedLogicalSubscription,
    sink: Parameters<DocumentConnectionPool["subscribe"]>[1],
    pendingAuthorization: ReauthorizedLogicalSubscription | null = null,
  ): LogicalSubscriptionHandle {
    if (this.#pool === null) throw new Error("async_stream_configuration_missing");
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

  retirePoll(controller: PollingIslandController): void {
    this.#pollControllers.delete(controller);
  }

  suspend(): void {
    if (this.#state !== "active" && this.#state !== "resuming") return;
    this.#state = "suspended";
    this.#authorizationScheduler.pause();
    for (const controller of this.#controllers) controller.suspend();
    for (const controller of this.#pollControllers) controller.suspend();
    this.#pool?.suspend();
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
    for (const controller of this.#pollControllers) controller.resume();
    await Promise.all([...(this.#pool === null ? [] : [this.#pool.resume()]), ...initial]);
    if (this.#resuming()) this.#state = "active";
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#authorizationScheduler.pause();
    for (const controller of [...this.#controllers]) controller.dispose();
    for (const controller of [...this.#pollControllers]) controller.dispose();
    this.#pool?.dispose();
  }

  #resuming(): boolean {
    return this.#state === "resuming";
  }
}

function configuredFeature(options: () => AsyncFeatureOptions | null): RuntimeFeature {
  const sharedHandshakes = new OriginHandshakeScheduler();
  const definition: AsyncRuntimeFeatureDefinition = Object.freeze({
    connectDocument(context: RuntimeFeatureDocumentContext) {
      const configured = options();
      if (configured === null) {
        return Object.freeze({
          connectIsland(port: AsyncRuntimeIslandPort) {
            if (
              port
                .queryDirectiveOwnership(parseFeatureDirective)
                .some(({ directive }) => directive.name === "stream" || directive.name === "poll")
            ) {
              report(context, "operation_rejected");
            }
            return undefined;
          },
          dispose() {
            // No resources exist before the application supplies async clocks and ports.
          },
        });
      }
      const owner = new AsyncDocumentOwner(context, {
        ...configured,
        handshakeScheduler: configured.handshakeScheduler ?? sharedHandshakes,
      });
      return Object.freeze({
        connectIsland(port: AsyncRuntimeIslandPort) {
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
