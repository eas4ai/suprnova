export interface AsyncWorkloadInput {
  readonly artifactUrl: string;
  readonly expectedArtifactSha256: string;
  readonly eventEnvelopeBytes: 1_024;
  readonly multiDocumentCount: 16;
  readonly presentationEventCount: 1_000;
  readonly refreshInvalidationCount: 100;
  readonly scheduledDurationMs: 10_000;
  readonly subscriptionCount: 100;
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
    readonly multiDocument: Readonly<{
      readonly documentCount: 16;
      readonly completedHandshakes: number;
      readonly maximumConcurrentHandshakes: number;
    }>;
  }>;
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
  connectIsland(port: Readonly<Record<string, unknown>>): { dispose(): void } | undefined;
  dispose(): void;
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
export function measureAsyncWorkloads(input: AsyncWorkloadInput): Promise<AsyncWorkloadMeasurement>;
export async function measureAsyncWorkloads(
  input: AsyncWorkloadInput | AsyncWorkloadPreparationInput,
): Promise<AsyncWorkloadMeasurement | AsyncWorkloadPreparation> {
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
  }

  class MeasuredSource implements BenchmarkSource {
    readonly subscriptions = new Set<string>();
    closed = false;

    constructor(readonly request: BenchmarkConnectRequest) {}

    get membershipCount(): number {
      return this.subscriptions.size;
    }

    close(): void {
      this.closed = true;
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
      this.subscriptions.add(subscription.subscriptionId);
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
  let recoveryStarted = 0;
  let activeReauthorizations = 0;
  let maximumConcurrentReauthorizations = 0;
  let handshakes = 0;
  let queuedEventPeak = 0;
  let queuedBytePeak = 0;
  let maximumQueuedRefreshesPerIsland = 0;
  let maximumInFlightRefreshesPerIsland = 0;

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
        queuedEventPeak = Math.max(queuedEventPeak, queuedEvents);
        queuedBytePeak = Math.max(queuedBytePeak, queuedBytes);
        maximumQueuedRefreshesPerIsland = Math.max(
          maximumQueuedRefreshesPerIsland,
          queuedRefreshes,
        );
        maximumInFlightRefreshesPerIsland = Math.max(
          maximumInFlightRefreshesPerIsland,
          inFlightRefreshes,
        );
      },
      pollEnvironment: Object.freeze({
        isOnline: () => true,
        isVisible: () => true,
        subscribe: () => () => undefined,
      }),
      randomness: Object.freeze({ number: () => 0.5 }),
      timers: timers.port,
      transports: Object.freeze({
        eventSource(request: BenchmarkConnectRequest) {
          handshakes += 1;
          const source = new MeasuredSource(request);
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

  for (let index = 0; index < input.subscriptionCount; index += 1) {
    const root = document.querySelector(`[data-async-benchmark-index="${String(index)}"]`);
    if (!(root instanceof HTMLElement)) throw new Error("e100_island_missing");
    pendingStarts.set(index, []);
    refreshCompletions.set(index, []);
    const stream = `benchmark-${String(index).padStart(3, "0")}`;
    owner.connectIsland(
      Object.freeze({
        consumeRegisteredEventCapability: () => Object.freeze({}),
        dispatchRegisteredEvent: (_capability: unknown, candidate: unknown) => {
          const event = candidate as Readonly<{ event: string; payload: unknown }>;
          const started = pendingStarts.get(index)?.shift();
          if (started === undefined) throw new Error("e100_dispatch_start_missing");
          root.dispatchEvent(new CustomEvent(event.event, { detail: event.payload }));
          dispatchEffectSamplesMs.push(performance.now() - started);
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
      }),
    );
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
  const presentationEnvelope = (index: number, sequence: number, eventIndex: number): string => {
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
    const padding = input.eventEnvelopeBytes - new TextEncoder().encode(empty).byteLength;
    if (padding < 0) throw new Error("e100_envelope_overflow");
    const encoded = encode("x".repeat(padding));
    if (new TextEncoder().encode(encoded).byteLength !== input.eventEnvelopeBytes) {
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
    pendingStarts.get(index)?.push(performance.now());
    primarySource.emit(presentationEnvelope(index, nextSequence(index), next));
    presentationEventCount = next;
  };
  const emitRefresh = (index: number): void => {
    primarySource.emit(refreshEnvelope(index, nextSequence(index)));
    refreshInvalidationCount += 1;
  };

  for (let round = 0; round < 5; round += 1) {
    for (let index = 0; index < input.subscriptionCount; index += 1) emitPresentation(index);
  }
  for (const first of [0, 25]) {
    for (let index = first; index < first + 25; index += 1) {
      emitRefresh(index);
      emitRefresh(index);
      emitPresentation(index);
    }
    for (let index = first; index < first + 25; index += 1) {
      const completion = refreshCompletions.get(index)?.shift();
      if (completion === undefined) throw new Error("e100_first_refresh_missing");
      completion("succeeded");
    }
    for (let index = first; index < first + 25; index += 1) {
      const completion = refreshCompletions.get(index)?.shift();
      if (completion === undefined) throw new Error("e100_second_refresh_missing");
      completion("succeeded");
    }
  }
  for (let index = 0; index < input.subscriptionCount; index += 1) {
    const remaining = index < 50 ? 4 : 5;
    for (let count = 0; count < remaining; count += 1) emitPresentation(index);
  }
  if (
    presentationEventCount !== input.presentationEventCount ||
    dispatchEffectSamplesMs.length !== input.presentationEventCount ||
    refreshInvalidationCount !== input.refreshInvalidationCount
  ) {
    throw new Error("e100_workload_shape_mismatch");
  }
  const currentBeforeRecovery = [...states.values()].filter((state) => state === "current").length;
  const scheduledDurationMs = timers.current;
  const physicalConnectionCount = sources.length;
  const initialHandshakeCount = handshakes;

  recoveryStarted = performance.now();
  primarySource.fail();
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

  const scheduler = new artifact.OriginHandshakeScheduler();
  const releases: VoidFunction[] = [];
  let completedHandshakes = 0;
  let maximumConcurrentHandshakes = 0;
  for (let index = 0; index < input.multiDocumentCount; index += 1) {
    scheduler.schedule(location.origin, (release) => {
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

  const pollingDue = new Map<number, number>();
  const pollingHandles = new Map<number, number>();
  let pollingHandle = 0;
  let pollingRandom = 0;
  const pollingOwner = new artifact.AsyncDocumentOwner(
    Object.freeze({ diagnose: () => undefined, onDispose: () => undefined }),
    Object.freeze({
      clock: Object.freeze({ now: () => 1_000 }),
      pollEnvironment: Object.freeze({
        isOnline: () => true,
        isVisible: () => true,
        subscribe: () => () => undefined,
      }),
      randomness: Object.freeze({
        number: () => {
          const value = (pollingRandom + 0.5) / input.subscriptionCount;
          pollingRandom += 1;
          return value;
        },
      }),
      timers: Object.freeze({
        clearTimeout(handle: number) {
          const due = pollingHandles.get(handle);
          if (due === undefined) return;
          pollingHandles.delete(handle);
          const count = pollingDue.get(due) ?? 0;
          if (count <= 1) pollingDue.delete(due);
          else pollingDue.set(due, count - 1);
        },
        timeout(_callback: VoidFunction, milliseconds: number) {
          pollingHandle += 1;
          pollingHandles.set(pollingHandle, milliseconds);
          pollingDue.set(milliseconds, (pollingDue.get(milliseconds) ?? 0) + 1);
          return pollingHandle;
        },
      }),
    }),
  );
  for (let index = 0; index < input.subscriptionCount; index += 1) {
    const root = document.querySelector(`[data-async-benchmark-index="${String(index)}"]`);
    if (!(root instanceof HTMLElement)) throw new Error("r100_poll_island_missing");
    pollingOwner.connectIsland(
      Object.freeze({
        consumeRegisteredEventCapability: () => Object.freeze({}),
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: () => "queued",
        identity: Object.freeze({
          component: "benchmark.poll",
          documentKey: `benchmark-poll-${String(index)}`,
          slot: `poll-${String(index)}`,
        }),
        onDispose: () => undefined,
        queryDirectiveOwnership: () =>
          Object.freeze([
            Object.freeze({
              attributeName: "live:poll",
              directive: Object.freeze({
                capability: "async@1",
                modifiers: Object.freeze([]),
                name: "poll",
                ok: true,
                role: null,
                value: "",
              }),
              element: root,
            }),
          ]),
        writePresentationSignal: (_scope: string, _name: string, value: unknown) => value,
      }),
    );
  }
  const pollingMaximumSameTick = Math.max(...pollingDue.values());
  pollingOwner.dispose();

  const recoveredSubscriptionCount = recoveryAt.size;
  const documentReconnectHandshakes = sources.length - physicalConnectionCount;
  const measuredDispatchEffects = Object.freeze([...dispatchEffectSamplesMs]);
  const measurement = Object.freeze({
    E100: Object.freeze({
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
    }),
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
      multiDocument: Object.freeze({
        documentCount: 16 as const,
        completedHandshakes,
        maximumConcurrentHandshakes,
      }),
    }),
  });
  // Evidence collectors are not product state and must not inflate the retained owner sample.
  authorizations.clear();
  dispatchEffectSamplesMs.length = 0;
  pendingStarts.clear();
  recoveryAt.clear();
  refreshCompletions.clear();
  sources.length = 0;
  states.clear();
  // The page closes immediately after the retained-heap read.
  Reflect.set(globalThis, "__suprnovaBudgetAsyncRetention", owner);
  return measurement;
}
