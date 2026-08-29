import type { AsyncSubscriptionOwnerObservation } from "./async-budget-workloads.js";

export interface AsyncWorkloadInput {
  readonly artifactUrl: string;
  readonly expectedArtifactSha256: string;
  readonly eventEnvelopeBytes: 1_024;
  readonly multiDocumentCount: 16;
  readonly presentationEventCount: 1_000;
  readonly refreshInvalidationCount: 100;
  readonly scheduledDurationMs: 10_000;
  readonly subscriptionCount: 100;
  readonly retentionMutation?:
    | "none"
    | "large_island_buffer"
    | "predecessor_transport"
    | "stale_current_payload"
    | "stale_queued_payload";
}

export interface AsyncWorkloadE100CheckpointInput extends AsyncWorkloadInput {
  readonly retentionCheckpoint: "e100";
}

export interface AsyncWorkloadR100CheckpointInput extends AsyncWorkloadInput {
  readonly retentionCheckpoint: "r100";
}

export interface AsyncWorkloadPreparationInput extends AsyncWorkloadInput {
  readonly prepare: true;
}

export interface AsyncWorkloadPreparation {
  readonly artifactSha256: string;
}

export interface AsyncWorkloadMeasurement {
  readonly E100: Readonly<{
    readonly artifactSha256: string;
    readonly dispatchEffectSamplesMs: readonly number[];
    readonly presentationEventCount: number;
    readonly refreshInvalidationCount: number;
    readonly scheduledDurationMs: number;
    readonly physicalConnectionCount: number;
    readonly handshakeCount: number;
    readonly queuedEventPeak: number;
    readonly queuedBytePeak: number;
    readonly maximumQueuedRefreshesPerIsland: number;
    readonly maximumInFlightRefreshesPerIsland: number;
    readonly currentSubscriptionCount: number;
    readonly fairnessMaximumLead: number;
    readonly subscriptions: readonly Readonly<{
      readonly current: boolean;
      readonly dispatches: number;
      readonly finalEpoch: string;
      readonly finalSequence: string;
      readonly id: string;
      readonly maxInFlightRefreshes: number;
      readonly maxQueuedRefreshes: number;
      readonly presentationEvents: number;
      readonly refreshInvalidations: number;
    }>[];
  }>;
  readonly R100: Readonly<{
    readonly artifactSha256: string;
    readonly documentReconnectHandshakes: number;
    readonly recoverySamplesMs: readonly number[];
    readonly maximumRecoverySkewMs: number;
    readonly recoveredSubscriptionCount: number;
    readonly currentSubscriptionCount: number;
    readonly starvedSubscriptionCount: number;
    readonly maximumConcurrentReauthorizations: number;
    readonly pollingMaximumSameTick: number;
    readonly pollDueMilliseconds: readonly number[];
    readonly reconnectDelayMilliseconds: number;
    readonly generationBefore: number;
    readonly generationAfter: number;
    readonly physicalTransportsAfterCurrent: number;
    readonly predecessorContinuityOwners: number;
    readonly predecessorTransportOwners: number;
    readonly queuedPayloadOwners: number;
    readonly currentPayloadOwners: number;
    readonly recovery: readonly Readonly<{
      readonly current: boolean;
      readonly id: string;
      readonly jitterMilliseconds: number;
      readonly pollDueMilliseconds: number;
      readonly timeToCurrentMilliseconds: number;
    }>[];
    readonly multiDocument: Readonly<{
      readonly documentCount: 16;
      readonly completedHandshakes: number;
      readonly maximumConcurrentHandshakes: number;
      readonly startOrder: readonly number[];
    }>;
  }>;
}

export interface AsyncWorkloadE100Checkpoint {
  readonly E100: AsyncWorkloadMeasurement["E100"];
  readonly R100: null;
}

interface BenchmarkAuthorization {
  readonly authorization: Readonly<{ kind: "session_cookie" }>;
  readonly baseline: Readonly<{ epoch: bigint; sequence: bigint }>;
  readonly descriptorBinding: string;
  readonly document: Readonly<{
    authorizationScope: string;
    origin: string;
    transport: "sse";
  }>;
  readonly events: readonly Readonly<Record<string, unknown>>[];
  readonly expiresAt: number;
  readonly fallbackPoll: Readonly<Record<string, unknown>>;
  readonly heartbeatTimeoutMs: number;
  readonly presentationSignals: readonly Readonly<Record<string, unknown>>[];
  readonly reconnect: Readonly<Record<string, unknown>>;
  readonly stream: string;
  readonly subscriptionId: string;
}

interface BenchmarkConnectRequest {
  readonly transportGeneration: number;
  message(encoded: string): void;
  opened(): void;
  failed(reason: "transport_lost"): void;
}

interface BenchmarkSource {
  readonly membershipCount: number;
  emit(encoded: string): void;
  fail(): void;
  open(): void;
}

interface BenchmarkOwner {
  connectIsland(port: Readonly<Record<string, unknown>>): { dispose(): void };
  dispose(): void;
}

interface BenchmarkRetentionSession {
  readonly phase: "e100" | "r100";
  cleanup(): Promise<void>;
  resources(): Readonly<{
    activeTransportOwners: number;
    currentPayloadOwners: number;
    predecessorContinuityOwners: number;
    predecessorTransportOwners: number;
    queuedPayloadOwners: number;
    subscriptions: readonly AsyncSubscriptionOwnerObservation[];
  }>;
}

interface BenchmarkAsyncArtifact {
  readonly AsyncDocumentOwner: new (
    context: Readonly<Record<string, unknown>>,
    options: Readonly<Record<string, unknown>>,
  ) => BenchmarkOwner;
  readonly OriginHandshakeScheduler: new () => {
    active(origin: string): number;
    schedule(origin: string, start: (release: VoidFunction) => void): unknown;
  };
}

/** Runs in Playwright; protocol behavior comes from the hashed dist module. */
export function measureAsyncWorkloads(
  input: AsyncWorkloadPreparationInput,
): Promise<AsyncWorkloadPreparation>;
export function measureAsyncWorkloads(
  input: AsyncWorkloadE100CheckpointInput,
): Promise<AsyncWorkloadE100Checkpoint>;
export function measureAsyncWorkloads(
  input: AsyncWorkloadInput | AsyncWorkloadR100CheckpointInput,
): Promise<AsyncWorkloadMeasurement>;
export async function measureAsyncWorkloads(
  input:
    | AsyncWorkloadInput
    | AsyncWorkloadE100CheckpointInput
    | AsyncWorkloadR100CheckpointInput
    | AsyncWorkloadPreparationInput,
): Promise<AsyncWorkloadE100Checkpoint | AsyncWorkloadMeasurement | AsyncWorkloadPreparation> {
  function canonicalize(value: unknown): string {
    const normalize = (candidate: unknown): unknown => {
      if (Array.isArray(candidate)) return candidate.map(normalize);
      if (typeof candidate !== "object" || candidate === null) return candidate;
      const record = candidate as Record<string, unknown>;
      return Object.fromEntries(
        Object.keys(record)
          .sort()
          .map((key) => [key, normalize(record[key])]),
      );
    };
    const encoded = JSON.stringify(normalize(value));
    return encoded;
  }

  class ControlledTimers {
    readonly pending = new Map<number, Readonly<{ callback: VoidFunction; due: number }>>();
    next = 0;
    current = 0;

    readonly port = Object.freeze({
      clearTimeout: (handle: number) => {
        this.pending.delete(handle);
      },
      timeout: (callback: VoidFunction, milliseconds: number) => {
        this.next += 1;
        this.pending.set(this.next, Object.freeze({ callback, due: this.current + milliseconds }));
        return this.next;
      },
    });

    advanceTo(milliseconds: number): void {
      if (!Number.isFinite(milliseconds) || milliseconds < this.current) {
        throw new Error("async_benchmark_clock_regression");
      }
      this.current = milliseconds;
    }

    fireEarliest(): void {
      const next = [...this.pending].sort(
        (left, right) => left[1].due - right[1].due || left[0] - right[0],
      )[0];
      if (next === undefined) throw new Error("r100_reconnect_timer_missing");
      this.pending.delete(next[0]);
      this.current = next[1].due;
      next[1].callback();
    }

    scheduledCountAfter(minimumDue: number): number {
      return [...this.pending.values()].filter(({ due }) => due >= minimumDue).length;
    }

    maximumSameDueAfter(minimumDue: number): number {
      const counts = new Map<number, number>();
      for (const { due } of this.pending.values()) {
        if (due < minimumDue) continue;
        counts.set(due, (counts.get(due) ?? 0) + 1);
      }
      return Math.max(0, ...counts.values());
    }

    dueAfter(minimumDue: number): readonly number[] {
      return Object.freeze(
        [...this.pending.values()]
          .map(({ due }) => due)
          .filter((due) => due >= minimumDue)
          .sort((left, right) => left - right),
      );
    }

    earliestDueBefore(maximumDue: number): number {
      const due = [...this.pending.values()]
        .map((entry) => entry.due)
        .filter((candidate) => candidate < maximumDue)
        .sort((left, right) => left - right)[0];
      if (due === undefined) throw new Error("r100_reconnect_timer_missing");
      return due;
    }
  }

  class MeasuredSource implements BenchmarkSource {
    readonly subscriptions = new Map<string, BenchmarkAuthorization>();
    closed = false;
    private requestValue: BenchmarkConnectRequest | null;

    constructor(
      request: BenchmarkConnectRequest,
      private readonly retainAfterClose: boolean,
    ) {
      this.requestValue = request;
    }

    get request(): BenchmarkConnectRequest {
      if (this.requestValue === null) throw new Error("async_benchmark_source_released");
      return this.requestValue;
    }

    get membershipCount(): number {
      return this.subscriptions.size;
    }

    close(): void {
      if (this.retainAfterClose) return;
      this.release();
    }

    release(): void {
      this.closed = true;
      this.subscriptions.clear();
      this.requestValue = null;
    }

    emit(encoded: string): void {
      if (this.closed) throw new Error("async_benchmark_source_closed");
      this.request.message(encoded);
    }

    fail(): void {
      if (this.closed) throw new Error("async_benchmark_source_closed");
      this.request.failed("transport_lost");
    }

    open(): void {
      if (this.closed) throw new Error("async_benchmark_source_closed");
      this.request.opened();
    }

    subscribe(subscription: BenchmarkAuthorization): Readonly<Record<string, unknown>> {
      this.subscriptions.set(subscription.subscriptionId, subscription);
      return Object.freeze({
        descriptorBinding: subscription.descriptorBinding,
        kind: "authenticated",
        stream: subscription.stream,
        subscriptionId: subscription.subscriptionId,
        transportGeneration: this.request.transportGeneration,
      });
    }

    unsubscribe(subscriptionId: string): void {
      this.subscriptions.delete(subscriptionId);
    }
  }

  const settleUntil = async (predicate: () => boolean, code: string): Promise<void> => {
    for (let turn = 0; turn < 2_048; turn += 1) {
      if (predicate()) return;
      await Promise.resolve();
    }
    throw new Error(code);
  };

  const artifactUrl = input.artifactUrl;
  const prepared = Reflect.get(globalThis, "__suprnovaBudgetAsyncPrepared") as
    | Readonly<{
        artifact: BenchmarkAsyncArtifact;
        artifactSha256: string;
        artifactUrl: string;
      }>
    | undefined;
  let artifact: BenchmarkAsyncArtifact;
  let artifactSha256: string;
  if (
    prepared?.artifactUrl === artifactUrl &&
    prepared.artifactSha256 === input.expectedArtifactSha256
  ) {
    artifact = prepared.artifact;
    artifactSha256 = prepared.artifactSha256;
  } else {
    const artifactResponse = await fetch(artifactUrl, { cache: "no-store" });
    if (!artifactResponse.ok) throw new Error("async_benchmark_artifact_fetch_failed");
    const artifactBytes = await artifactResponse.arrayBuffer();
    const artifactDigest = await crypto.subtle.digest("SHA-256", artifactBytes);
    artifactSha256 = [...new Uint8Array(artifactDigest)]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
    if (artifactSha256 !== input.expectedArtifactSha256) {
      throw new Error("async_benchmark_artifact_hash_mismatch");
    }
    const loaded: unknown = await import(artifactUrl);
    artifact = loaded as BenchmarkAsyncArtifact;
  }
  if (
    typeof artifact.AsyncDocumentOwner !== "function" ||
    typeof artifact.OriginHandshakeScheduler !== "function"
  ) {
    throw new Error("async_benchmark_artifact_exports_missing");
  }
  if ("prepare" in input) {
    Reflect.set(
      globalThis,
      "__suprnovaBudgetAsyncPrepared",
      Object.freeze({ artifact, artifactSha256, artifactUrl }),
    );
    Reflect.set(globalThis, "__suprnovaBudgetAsyncMeasure", measureAsyncWorkloads);
    return Object.freeze({ artifactSha256 });
  }

  const timers = new ControlledTimers();
  const sources: MeasuredSource[] = [];
  const authorizations = new Map<string, BenchmarkAuthorization>();
  const states = new Map<string, string>();
  const recoveryAt = new Map<string, number>();
  const pendingStarts = new Map<number, number[]>();
  const refreshCompletions = new Map<number, ((completion: "succeeded") => void)[]>();
  const dispatchEffectSamplesMs: number[] = [];
  const presentationCounts = new Array<number>(input.subscriptionCount).fill(0);
  const refreshCounts = new Array<number>(input.subscriptionCount).fill(0);
  const dispatchCounts = new Array<number>(input.subscriptionCount).fill(0);
  const maxQueuedRefreshes = new Array<number>(input.subscriptionCount).fill(0);
  const maxInFlightRefreshes = new Array<number>(input.subscriptionCount).fill(0);
  const currentInFlightRefreshes = new Array<number>(input.subscriptionCount).fill(0);
  const currentPayloadBytes = new Array<number>(input.subscriptionCount).fill(0);
  const currentQueuedRefreshes = new Array<number>(input.subscriptionCount).fill(0);
  const queuedPayloadBytes = new Array<number>(input.subscriptionCount).fill(0);
  let recoveryStarted = 0;
  let activeReauthorizations = 0;
  let maximumConcurrentReauthorizations = 0;
  let handshakes = 0;
  let queuedEventPeak = 0;
  let queuedBytePeak = 0;
  let currentQueuedEvents = 0;
  let currentQueuedBytes = 0;
  let maximumQueuedRefreshesPerIsland = 0;
  let maximumInFlightRefreshesPerIsland = 0;
  let pollingRandom = 0;
  let fairnessMaximumLead = 0;
  let observedQueueIndex: number | null = null;
  let observedPayloadBytes = 0;
  const withObservedQueueOwner = <Result>(
    index: number,
    encodedBytes: number,
    operation: () => Result,
  ): Result => {
    if (observedQueueIndex !== null) throw new Error("async_benchmark_queue_owner_overlap");
    observedQueueIndex = index;
    observedPayloadBytes = encodedBytes;
    try {
      return operation();
    } finally {
      observedQueueIndex = null;
      observedPayloadBytes = 0;
    }
  };

  const makeAuthorization = (
    index: number,
    baseline: Readonly<{ epoch: bigint; sequence: bigint }>,
    reconnect: boolean,
  ): BenchmarkAuthorization => {
    const suffix = String(index).padStart(3, "0");
    return Object.freeze({
      authorization: Object.freeze({ kind: "session_cookie" as const }),
      baseline,
      descriptorBinding: `benchmark-binding-${suffix}${reconnect ? "-reconnected" : ""}`,
      document: Object.freeze({
        authorizationScope: "benchmark-shared-document",
        origin: location.origin,
        transport: "sse" as const,
      }),
      events: Object.freeze([
        Object.freeze({
          cycle: Object.freeze({ kind: "forbid_repeated_island" }),
          maximumFanout: 1,
          name: "benchmark.presented",
          order: "per_source_sequence",
          payloadContract: "benchmark.presented.v1",
          schema: "json",
          source: "stream",
          targets: Object.freeze(["self"]),
          version: 1,
        }),
      ]),
      expiresAt: Number.MAX_SAFE_INTEGER,
      fallbackPoll: Object.freeze({
        initial: "wait",
        intervalMs: 30_000,
        jitterRatio: 0.2,
        visibility: "visible",
      }),
      heartbeatTimeoutMs: 30_000,
      presentationSignals: Object.freeze([]),
      reconnect: Object.freeze({
        kind: "resume_or_refresh",
        maximumAttempts: 4,
        maximumDelayMs: 30_000,
        minimumDelayMs: 250,
      }),
      stream: `benchmark-${suffix}`,
      subscriptionId: `subscription-${suffix}`,
    });
  };

  const authority = Object.freeze({
    async authorize(request: Readonly<Record<string, unknown>>) {
      const identity = request["identity"] as Readonly<Record<string, unknown>>;
      const slot = String(identity["slot"]);
      const index = Number.parseInt(slot.slice("benchmark-".length), 10);
      if (!Number.isSafeInteger(index) || index < 0 || index >= input.subscriptionCount) {
        throw new Error("async_benchmark_authority_identity_invalid");
      }
      const prior = request["prior"] as BenchmarkAuthorization | null;
      if (prior !== null) {
        activeReauthorizations += 1;
        maximumConcurrentReauthorizations = Math.max(
          maximumConcurrentReauthorizations,
          activeReauthorizations,
        );
      }
      await Promise.resolve();
      if (prior !== null) activeReauthorizations -= 1;
      const position = request["position"] as Readonly<{ epoch: bigint; sequence: bigint }> | null;
      const current = makeAuthorization(
        index,
        position ?? Object.freeze({ epoch: 1n, sequence: 0n }),
        prior !== null,
      );
      authorizations.set(current.subscriptionId, current);
      return Object.freeze({ replay: Object.freeze([]), subscription: current });
    },
  });

  const owner = new artifact.AsyncDocumentOwner(
    Object.freeze({ diagnose: () => undefined, onDispose: () => undefined }),
    Object.freeze({
      authority,
      clock: Object.freeze({ now: () => 1_000 }),
      observeQueuePressure(observation: Readonly<Record<string, unknown>>) {
        const queuedEvents = Number(observation["documentQueuedEvents"]);
        const queuedBytes = Number(observation["documentQueuedBytes"]);
        const queuedRefreshes = Number(observation["islandQueuedRefreshes"]);
        const inFlightRefreshes = Number(observation["islandInFlightRefreshes"]);
        if (
          ![queuedEvents, queuedBytes, queuedRefreshes, inFlightRefreshes].every(
            (value) => Number.isSafeInteger(value) && value >= 0,
          )
        ) {
          throw new Error("async_benchmark_queue_observation_invalid");
        }
        const previousQueuedBytes = currentQueuedBytes;
        queuedEventPeak = Math.max(queuedEventPeak, queuedEvents);
        queuedBytePeak = Math.max(queuedBytePeak, queuedBytes);
        currentQueuedEvents = queuedEvents;
        currentQueuedBytes = queuedBytes;
        maximumQueuedRefreshesPerIsland = Math.max(
          maximumQueuedRefreshesPerIsland,
          queuedRefreshes,
        );
        maximumInFlightRefreshesPerIsland = Math.max(
          maximumInFlightRefreshesPerIsland,
          inFlightRefreshes,
        );
        const index = observedQueueIndex;
        if (index === null) {
          throw new Error("async_benchmark_queue_owner_invalid");
        }
        const previousInFlight = currentInFlightRefreshes[index] ?? 0;
        const queuedDelta = queuedBytes - previousQueuedBytes;
        const nextIslandQueuedBytes = (queuedPayloadBytes[index] ?? 0) + queuedDelta;
        if (!Number.isSafeInteger(nextIslandQueuedBytes) || nextIslandQueuedBytes < 0) {
          throw new Error("async_benchmark_queue_byte_owner_invalid");
        }
        queuedPayloadBytes[index] = nextIslandQueuedBytes;
        currentQueuedRefreshes[index] = queuedRefreshes;
        currentInFlightRefreshes[index] = inFlightRefreshes;
        if (previousInFlight === 0 && inFlightRefreshes > 0) {
          currentPayloadBytes[index] = observedPayloadBytes;
        } else if (previousInFlight > 0 && inFlightRefreshes === 0) {
          currentPayloadBytes[index] = 0;
        }
        maxQueuedRefreshes[index] = Math.max(maxQueuedRefreshes[index] ?? 0, queuedRefreshes);
        maxInFlightRefreshes[index] = Math.max(maxInFlightRefreshes[index] ?? 0, inFlightRefreshes);
      },
      pollEnvironment: Object.freeze({
        isOnline: () => true,
        isVisible: () => true,
        subscribe: () => () => undefined,
      }),
      randomness: Object.freeze({
        number: () => {
          const value = ((pollingRandom % input.subscriptionCount) + 0.5) / input.subscriptionCount;
          pollingRandom += 1;
          return value;
        },
      }),
      timers: timers.port,
      transports: Object.freeze({
        eventSource(request: BenchmarkConnectRequest) {
          handshakes += 1;
          const source = new MeasuredSource(
            request,
            input.retentionMutation === "predecessor_transport",
          );
          sources.push(source);
          return source;
        },
        webSocket() {
          throw new Error("async_benchmark_unexpected_websocket");
        },
      }),
    }),
  );

  const streamOwnership = (root: Element, stream: string) =>
    Object.freeze([
      Object.freeze({
        attributeName: "live:stream",
        directive: Object.freeze({
          capability: "async@1",
          modifiers: Object.freeze([]),
          name: "stream",
          ok: true,
          role: null,
          value: stream,
        }),
        element: root,
      }),
    ]);

  const handles: ({ dispose(): void } | null)[] = Array.from(
    { length: input.subscriptionCount },
    () => null,
  );
  const connectIsland = (index: number): void => {
    if (handles[index] !== null) throw new Error("async_benchmark_duplicate_island");
    const root = document.querySelector(`[data-async-benchmark-index="${String(index)}"]`);
    if (!(root instanceof HTMLElement)) throw new Error("e100_island_missing");
    pendingStarts.set(index, pendingStarts.get(index) ?? []);
    refreshCompletions.set(index, refreshCompletions.get(index) ?? []);
    const stream = `benchmark-${String(index).padStart(3, "0")}`;
    const port = Object.freeze({
      consumeRegisteredEventCapability: () => Object.freeze({}),
      dispatchRegisteredEvent: (_capability: unknown, candidate: unknown) => {
        const event = candidate as Readonly<{ event: string; payload: unknown }>;
        const started = pendingStarts.get(index)?.shift();
        if (started === undefined) throw new Error("e100_dispatch_start_missing");
        root.dispatchEvent(new CustomEvent(event.event, { detail: event.payload }));
        dispatchEffectSamplesMs.push(performance.now() - started);
        dispatchCounts[index] = (dispatchCounts[index] ?? 0) + 1;
        fairnessMaximumLead = Math.max(
          fairnessMaximumLead,
          Math.max(...dispatchCounts) - Math.min(...dispatchCounts),
        );
        return "dispatched";
      },
      element: root,
      enqueueFreshRender: (_reason: unknown, completion?: (outcome: "succeeded") => void) => {
        if (completion === undefined) throw new Error("e100_refresh_completion_missing");
        refreshCompletions.get(index)?.push(completion);
        return "queued";
      },
      identity: Object.freeze({
        component: "benchmark.component",
        documentKey: "benchmark-document",
        slot: `benchmark-${String(index)}`,
      }),
      onDispose: () => undefined,
      projectAsyncStatus: (state: string) => {
        const subscriptionId = `subscription-${String(index).padStart(3, "0")}`;
        states.set(subscriptionId, state);
        if (recoveryStarted > 0 && state === "current" && !recoveryAt.has(subscriptionId)) {
          recoveryAt.set(subscriptionId, performance.now() - recoveryStarted);
        }
      },
      queryDirectiveOwnership: () => streamOwnership(root, stream),
      writePresentationSignal: (_scope: string, _name: string, value: unknown) => value,
    });
    handles[index] = owner.connectIsland(port);
  };

  const installRetentionSession = (
    phase: "e100" | "r100",
    activeSource: MeasuredSource,
    predecessorSource: MeasuredSource | null,
  ): void => {
    if (
      input.retentionMutation === "none" &&
      (currentQueuedEvents !== 0 || currentQueuedBytes !== 0)
    ) {
      throw new Error("async_benchmark_retention_queue_not_drained");
    }
    if (
      input.retentionMutation === "none" &&
      [...pendingStarts.values()].some((entries) => entries.length !== 0)
    ) {
      throw new Error("async_benchmark_retention_payload_not_released");
    }
    const authorizationBytes = (index: number): number => {
      const value = authorizations.get(`subscription-${String(index).padStart(3, "0")}`);
      if (value === undefined) return 0;
      return new TextEncoder().encode(
        JSON.stringify(value, (_key: string, candidate: unknown): unknown =>
          typeof candidate === "bigint" ? candidate.toString() : candidate,
        ),
      ).byteLength;
    };
    const session: BenchmarkRetentionSession = Object.freeze({
      phase,
      async cleanup() {
        for (let index = 0; index < handles.length; index += 1) {
          withObservedQueueOwner(index, 0, () => {
            handles[index]?.dispose();
          });
          handles[index] = null;
        }
        owner.dispose();
        for (const source of sources) source.release();
        authorizations.clear();
        states.clear();
        recoveryAt.clear();
        pendingStarts.clear();
        refreshCompletions.clear();
        currentQueuedEvents = 0;
        currentQueuedBytes = 0;
        await settleUntil(
          () =>
            sources.every((source) => source.closed && source.membershipCount === 0) &&
            handles.every((handle) => handle === null),
          "async_benchmark_retention_cleanup_incomplete",
        );
      },
      resources() {
        const subscriptions = Object.freeze(
          Array.from({ length: input.subscriptionCount }, (_, index) =>
            Object.freeze({
              authorizationBytes: authorizationBytes(index),
              currentPayloadBytes: currentPayloadBytes[index] ?? 0,
              currentPayloadOwners: Number((currentInFlightRefreshes[index] ?? 0) > 0),
              id: `subscription-${String(index).padStart(3, "0")}`,
              queuedPayloadBytes: queuedPayloadBytes[index] ?? 0,
              queuedPayloadOwners: Number(
                (currentQueuedRefreshes[index] ?? 0) > 0 || (queuedPayloadBytes[index] ?? 0) > 0,
              ),
            }),
          ),
        );
        const predecessorTransportOwners =
          predecessorSource === null ? 0 : Number(!predecessorSource.closed);
        const predecessorContinuityOwners =
          predecessorSource === null ? 0 : predecessorSource.membershipCount;
        return Object.freeze({
          activeTransportOwners: sources.filter((source) => !source.closed).length,
          currentPayloadOwners: subscriptions.reduce(
            (sum, subscription) => sum + subscription.currentPayloadOwners,
            0,
          ),
          predecessorContinuityOwners,
          predecessorTransportOwners,
          queuedPayloadOwners: subscriptions.reduce(
            (sum, subscription) => sum + subscription.queuedPayloadOwners,
            0,
          ),
          subscriptions,
        });
      },
    });
    Reflect.set(globalThis, "__suprnovaBudgetAsyncRetention", session);
  };

  if ("retentionCheckpoint" in input) {
    const gate = Reflect.get(globalThis, "__suprnovaBudgetAsyncRetentionGate") as
      Readonly<{ wait(): Promise<void> }> | undefined;
    if (typeof gate?.wait !== "function") {
      throw new Error("async_benchmark_retention_gate_missing");
    }
    await gate.wait();
  }

  for (let index = 0; index < input.subscriptionCount; index += 1) {
    connectIsland(index);
  }

  await settleUntil(() => sources.length === 1, "e100_physical_connection_count");
  const primarySource = sources[0];
  if (primarySource === undefined) throw new Error("e100_source_missing");
  primarySource.open();
  await settleUntil(
    () => primarySource.membershipCount === input.subscriptionCount,
    "e100_membership_count",
  );

  const sequences = new Array<number>(input.subscriptionCount).fill(0);
  let presentationEventCount = 0;
  let refreshInvalidationCount = 0;
  const nextSequence = (index: number): number => {
    const next = (sequences[index] ?? 0) + 1;
    sequences[index] = next;
    return next;
  };
  const membership = (index: number): BenchmarkAuthorization => {
    const value = authorizations.get(`subscription-${String(index).padStart(3, "0")}`);
    if (value === undefined) throw new Error("e100_membership_missing");
    return value;
  };
  const presentationEnvelope = (
    index: number,
    sequence: number,
    eventIndex: number,
    targetBytes: number = input.eventEnvelopeBytes,
  ): string => {
    const current = membership(index);
    const encode = (padding: string): string =>
      canonicalize({
        payload: {
          event: "benchmark.presented",
          kind: "browser_event",
          payload: { index: eventIndex, padding },
          schema_version: 1,
          target: "self",
        },
        position: { epoch: "1", sequence: String(sequence) },
        protocol_version: 1,
        stream: current.stream,
        subscription: current.subscriptionId,
      });
    const empty = encode("");
    const padding = targetBytes - new TextEncoder().encode(empty).byteLength;
    if (padding < 0) throw new Error("e100_envelope_overflow");
    const encoded = encode("x".repeat(padding));
    if (new TextEncoder().encode(encoded).byteLength !== targetBytes) {
      throw new Error("e100_envelope_size");
    }
    return encoded;
  };
  const refreshEnvelope = (index: number, sequence: number): string => {
    const current = membership(index);
    return canonicalize({
      payload: { kind: "refresh", name: "refresh" },
      position: { epoch: "1", sequence: String(sequence) },
      protocol_version: 1,
      stream: current.stream,
      subscription: current.subscriptionId,
    });
  };
  const emitPresentation = (index: number): void => {
    const next = presentationEventCount + 1;
    timers.advanceTo((next * input.scheduledDurationMs) / input.presentationEventCount);
    const encoded = presentationEnvelope(index, nextSequence(index), next);
    pendingStarts.get(index)?.push(performance.now());
    withObservedQueueOwner(index, new TextEncoder().encode(encoded).byteLength, () => {
      primarySource.emit(encoded);
    });
    presentationCounts[index] = (presentationCounts[index] ?? 0) + 1;
    presentationEventCount = next;
  };
  const emitRefresh = (index: number): void => {
    const encoded = refreshEnvelope(index, nextSequence(index));
    withObservedQueueOwner(index, new TextEncoder().encode(encoded).byteLength, () => {
      primarySource.emit(encoded);
    });
    refreshCounts[index] = (refreshCounts[index] ?? 0) + 1;
    refreshInvalidationCount += 1;
  };

  for (let round = 0; round < 5; round += 1) {
    for (let index = 0; index < input.subscriptionCount; index += 1) emitPresentation(index);
  }
  for (let first = 0; first < input.subscriptionCount; first += 32) {
    const end = Math.min(input.subscriptionCount, first + 32);
    for (let index = first; index < end; index += 1) {
      emitRefresh(index);
      emitPresentation(index);
    }
    for (let index = first; index < end; index += 1) {
      const completion = refreshCompletions.get(index)?.shift();
      if (completion === undefined) throw new Error("e100_refresh_missing");
      withObservedQueueOwner(index, 0, () => {
        completion("succeeded");
      });
    }
  }
  for (let count = 0; count < 4; count += 1) {
    for (let index = 0; index < input.subscriptionCount; index += 1) {
      emitPresentation(index);
    }
  }
  if (
    presentationEventCount !== input.presentationEventCount ||
    dispatchEffectSamplesMs.length !== input.presentationEventCount ||
    refreshInvalidationCount !== input.refreshInvalidationCount
  ) {
    throw new Error("e100_workload_shape_mismatch");
  }
  const currentBeforeRecovery = [...states.values()].filter((state) => state === "current").length;
  const subscriptions = Object.freeze(
    Array.from({ length: input.subscriptionCount }, (_, index) => {
      const current = membership(index);
      return Object.freeze({
        current: states.get(current.subscriptionId) === "current",
        dispatches: dispatchCounts[index] ?? 0,
        finalEpoch: "1",
        finalSequence: String(sequences[index] ?? 0),
        id: current.subscriptionId,
        maxInFlightRefreshes: maxInFlightRefreshes[index] ?? 0,
        maxQueuedRefreshes: maxQueuedRefreshes[index] ?? 0,
        presentationEvents: presentationCounts[index] ?? 0,
        refreshInvalidations: refreshCounts[index] ?? 0,
      });
    }),
  );
  const scheduledDurationMs = timers.current;
  const physicalConnectionCount = sources.length;
  const initialHandshakeCount = handshakes;
  const generationBefore = primarySource.request.transportGeneration;
  const measuredDispatchEffects = Object.freeze([...dispatchEffectSamplesMs]);
  const e100Measurement = Object.freeze({
    artifactSha256,
    dispatchEffectSamplesMs: measuredDispatchEffects,
    presentationEventCount,
    refreshInvalidationCount,
    scheduledDurationMs,
    physicalConnectionCount,
    handshakeCount: initialHandshakeCount,
    queuedEventPeak,
    queuedBytePeak,
    maximumQueuedRefreshesPerIsland,
    maximumInFlightRefreshesPerIsland,
    currentSubscriptionCount: currentBeforeRecovery,
    fairnessMaximumLead,
    subscriptions,
  });

  const applyRetentionMutation = (activeSource: MeasuredSource, phase: "e100" | "r100"): void => {
    const mutation = input.retentionMutation ?? "none";
    const holdCurrent = phase === "r100" && mutation === "stale_current_payload";
    const holdQueued =
      (phase === "e100" && mutation === "large_island_buffer") ||
      (phase === "r100" && mutation === "stale_queued_payload");
    if (!holdCurrent && !holdQueued) return;
    const index = 0;
    const refresh = refreshEnvelope(index, nextSequence(index));
    withObservedQueueOwner(index, new TextEncoder().encode(refresh).byteLength, () => {
      activeSource.emit(refresh);
    });
    if (!holdQueued) return;
    for (let count = 0; count < 16; count += 1) {
      const queued = presentationEnvelope(index, nextSequence(index), 1_001 + count);
      pendingStarts.get(index)?.push(performance.now());
      withObservedQueueOwner(index, new TextEncoder().encode(queued).byteLength, () => {
        activeSource.emit(queued);
      });
    }
  };

  if ("retentionCheckpoint" in input && input.retentionCheckpoint === "e100") {
    applyRetentionMutation(primarySource, "e100");
    installRetentionSession("e100", primarySource, null);
    return Object.freeze({ E100: e100Measurement, R100: null });
  }

  recoveryStarted = performance.now();
  primarySource.fail();
  const pollingTimerMinimumDue = timers.current + 30_000;
  if (timers.scheduledCountAfter(pollingTimerMinimumDue) !== input.subscriptionCount) {
    throw new Error("r100_polling_membership_timer_count");
  }
  const pollingMaximumSameTick = timers.maximumSameDueAfter(pollingTimerMinimumDue);
  const pollDueMilliseconds = timers.dueAfter(pollingTimerMinimumDue);
  const reconnectDelayMilliseconds =
    timers.earliestDueBefore(pollingTimerMinimumDue) - timers.current;
  timers.fireEarliest();
  await settleUntil(() => sources.length === 2, "r100_document_reconnect_handshakes");
  const successorSource = sources[1];
  if (successorSource === undefined) throw new Error("r100_successor_source_missing");
  successorSource.open();
  await settleUntil(
    () =>
      successorSource.membershipCount === input.subscriptionCount &&
      recoveryAt.size === input.subscriptionCount,
    "r100_recovery_incomplete",
  );
  const recoverySamplesMs = Array.from(
    { length: input.subscriptionCount },
    (_, index) => recoveryAt.get(`subscription-${String(index).padStart(3, "0")}`) ?? Infinity,
  );
  const minimumRecovery = Math.min(...recoverySamplesMs);
  const maximumRecovery = Math.max(...recoverySamplesMs);
  const currentAfterRecovery = [...states.values()].filter((state) => state === "current").length;
  const generationAfter = successorSource.request.transportGeneration;
  const physicalTransportsAfterCurrent = sources.filter((source) => !source.closed).length;
  const predecessorTransportOwners = Number(!primarySource.closed);
  const predecessorContinuityOwners = primarySource.membershipCount;

  const scheduler = new artifact.OriginHandshakeScheduler();
  const releases: VoidFunction[] = [];
  const startOrder: number[] = [];
  let completedHandshakes = 0;
  let maximumConcurrentHandshakes = 0;
  for (let index = 0; index < input.multiDocumentCount; index += 1) {
    scheduler.schedule(location.origin, (release) => {
      startOrder.push(index);
      maximumConcurrentHandshakes = Math.max(
        maximumConcurrentHandshakes,
        scheduler.active(location.origin),
      );
      releases.push(() => {
        completedHandshakes += 1;
        release();
      });
    });
  }
  while (releases.length > 0) releases.shift()?.();

  const recoveredSubscriptionCount = recoveryAt.size;
  const documentReconnectHandshakes = sources.length - physicalConnectionCount;
  const measurement = Object.freeze({
    E100: e100Measurement,
    R100: Object.freeze({
      artifactSha256,
      documentReconnectHandshakes,
      recoverySamplesMs: Object.freeze(recoverySamplesMs),
      maximumRecoverySkewMs: maximumRecovery - minimumRecovery,
      recoveredSubscriptionCount,
      currentSubscriptionCount: currentAfterRecovery,
      starvedSubscriptionCount: input.subscriptionCount - recoveredSubscriptionCount,
      maximumConcurrentReauthorizations,
      pollingMaximumSameTick,
      pollDueMilliseconds: Object.freeze(pollDueMilliseconds),
      reconnectDelayMilliseconds,
      generationBefore,
      generationAfter,
      physicalTransportsAfterCurrent,
      predecessorContinuityOwners,
      predecessorTransportOwners,
      queuedPayloadOwners: currentQueuedEvents,
      currentPayloadOwners: 0,
      recovery: Object.freeze(
        Array.from({ length: input.subscriptionCount }, (_, index) => {
          const id = `subscription-${String(index).padStart(3, "0")}`;
          return Object.freeze({
            current: states.get(id) === "current",
            id,
            jitterMilliseconds: reconnectDelayMilliseconds,
            pollDueMilliseconds: pollDueMilliseconds[index] ?? 0,
            timeToCurrentMilliseconds: recoveryAt.get(id) ?? Infinity,
          });
        }),
      ),
      multiDocument: Object.freeze({
        documentCount: 16 as const,
        completedHandshakes,
        maximumConcurrentHandshakes,
        startOrder: Object.freeze(startOrder),
      }),
    }),
  });
  if ("retentionCheckpoint" in input) {
    applyRetentionMutation(successorSource, "r100");
    installRetentionSession("r100", successorSource, primarySource);
  }
  return measurement;
}
