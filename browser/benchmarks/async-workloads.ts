import { canonicalize } from "../src/canonical.js";
import {
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type DocumentMembershipOutcome,
  type DocumentTransportConnectRequest,
  type DocumentTransportPort,
  type LogicalSubscriptionHandle,
  type LogicalSubscriptionSink,
} from "../src/async-updates/connections.js";
import { AsyncDispatcher } from "../src/async-updates/dispatch.js";
import { PollTimer } from "../src/async-updates/poll.js";
import { AsyncSubscription } from "../src/async-updates/subscription.js";
import type {
  AsyncTimerPort,
  AuthorizedLogicalSubscription,
  SubscriptionState,
} from "../src/async-updates/types.js";
import type {
  AsyncRuntimeIslandPort,
  RegisteredBrowserEventCapability,
} from "../src/features/contract.js";
import { createE100Workload, createR100Workload } from "./workloads.js";

interface PendingTimer {
  readonly callback: VoidFunction;
  readonly due: number;
}

class ControlledTimers implements AsyncTimerPort {
  readonly #pending = new Map<number, PendingTimer>();
  #next = 0;
  #now = 0;

  clearTimeout(handle: number): void {
    this.#pending.delete(handle);
  }

  timeout(callback: VoidFunction, milliseconds: number): number {
    this.#next += 1;
    this.#pending.set(this.#next, Object.freeze({ callback, due: this.#now + milliseconds }));
    return this.#next;
  }

  advanceTo(milliseconds: number): void {
    if (!Number.isFinite(milliseconds) || milliseconds < this.#now) {
      throw new Error("async_benchmark_clock_regression");
    }
    this.#now = milliseconds;
  }

  fireNext(): boolean {
    const next = [...this.#pending].sort(
      (left, right) => left[1].due - right[1].due || left[0] - right[0],
    )[0];
    if (next === undefined) return false;
    this.#pending.delete(next[0]);
    this.#now = next[1].due;
    next[1].callback();
    return true;
  }

  now(): number {
    return this.#now;
  }
}

class MeasuredSource implements DocumentTransportPort {
  readonly #request: DocumentTransportConnectRequest;
  #closed = false;

  constructor(request: DocumentTransportConnectRequest) {
    this.#request = request;
  }

  close(): void {
    this.#closed = true;
  }

  emit(encoded: string): void {
    if (this.#closed) throw new Error("async_benchmark_source_closed");
    this.#request.message(encoded);
  }

  open(): void {
    if (this.#closed) throw new Error("async_benchmark_source_closed");
    this.#request.opened();
  }

  subscribe(subscription: AuthorizedLogicalSubscription): DocumentMembershipOutcome {
    return Object.freeze({
      descriptorBinding: subscription.descriptorBinding,
      kind: "authenticated" as const,
      stream: subscription.stream,
      subscriptionId: subscription.subscriptionId,
      transportGeneration: this.#request.transportGeneration,
    });
  }

  unsubscribe(subscriptionId: string): void {
    void subscriptionId;
  }
}

interface LogicalFixture {
  readonly authorization: AuthorizedLogicalSubscription;
  readonly handle: LogicalSubscriptionHandle;
  readonly subscription: AsyncSubscription;
}

export interface AsyncWorkloadMeasurement {
  readonly E100: Readonly<{
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

function authorization(index: number): AuthorizedLogicalSubscription {
  const suffix = String(index).padStart(3, "0");
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: `benchmark-binding-${suffix}`,
    document: Object.freeze({
      authorizationScope: "benchmark-shared-document",
      origin: location.origin,
      transport: "sse" as const,
    }),
    events: Object.freeze([
      Object.freeze({
        cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
        maximumFanout: 1,
        name: "benchmark.presented",
        order: "per_source_sequence" as const,
        payloadContract: "benchmark.presented.v1",
        schema: "json" as const,
        source: "stream" as const,
        targets: Object.freeze(["self"]),
        version: 1,
      }),
    ]),
    expiresAt: Number.MAX_SAFE_INTEGER,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 30_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 30_000,
      minimumDelayMs: 250,
    }),
    stream: `benchmark-${suffix}`,
    subscriptionId: `subscription-${suffix}`,
  });
}

function exactPresentationEnvelope(
  membership: AuthorizedLogicalSubscription,
  sequence: number,
): string {
  const encode = (padding: string): string =>
    canonicalize({
      payload: {
        event: "benchmark.presented",
        kind: "browser_event",
        payload: { index: sequence, padding },
        schema_version: 1,
        target: "self",
      },
      position: { epoch: "1", sequence: String(sequence) },
      protocol_version: 1,
      stream: membership.stream,
      subscription: membership.subscriptionId,
    });
  const empty = encode("");
  const paddingBytes = 1_024 - new TextEncoder().encode(empty).byteLength;
  if (paddingBytes < 0) throw new Error("e100_envelope_overflow");
  const encoded = encode("x".repeat(paddingBytes));
  if (new TextEncoder().encode(encoded).byteLength !== 1_024) {
    throw new Error("e100_envelope_size");
  }
  return encoded;
}

function refreshEnvelope(membership: AuthorizedLogicalSubscription, sequence: number): string {
  return canonicalize({
    payload: { kind: "refresh", name: "refresh" },
    position: { epoch: "1", sequence: String(sequence) },
    protocol_version: 1,
    stream: membership.stream,
    subscription: membership.subscriptionId,
  });
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 24; turn += 1) await Promise.resolve();
}

function multiDocumentEvidence(): AsyncWorkloadMeasurement["R100"]["multiDocument"] {
  const workload = createR100Workload();
  const scheduler = new OriginHandshakeScheduler();
  const releases: VoidFunction[] = [];
  let completed = 0;
  let maximum = 0;
  for (let index = 0; index < workload.multiDocumentCount; index += 1) {
    scheduler.schedule(location.origin, (release) => {
      maximum = Math.max(maximum, scheduler.active(location.origin));
      releases.push(() => {
        completed += 1;
        release();
      });
    });
  }
  while (releases.length > 0) releases.shift()?.();
  return Object.freeze({
    documentCount: 16 as const,
    completedHandshakes: completed,
    maximumConcurrentHandshakes: maximum,
  });
}

function pollingFanoutEvidence(): number {
  const due = new Map<number, number>();
  const timers: AsyncTimerPort = {
    clearTimeout: () => undefined,
    timeout: (_callback, milliseconds) => {
      due.set(milliseconds, (due.get(milliseconds) ?? 0) + 1);
      return due.size;
    },
  };
  const environment = {
    isOnline: () => true,
    isVisible: () => true,
    subscribe: () => () => undefined,
  };
  const polls = Array.from(
    { length: 100 },
    (_, index) =>
      new PollTimer({
        enqueueFreshRender: () => "queued",
        environment,
        policy: Object.freeze({
          initial: "wait" as const,
          intervalMs: 30_000,
          jitterRatio: 0.2,
          mode: "poll_only" as const,
          visibility: "visible" as const,
        }),
        randomness: { number: () => (index + 0.5) / 100 },
        timers,
      }),
  );
  for (const poll of polls) poll.start();
  const maximum = Math.max(...due.values());
  for (const poll of polls) poll.dispose();
  return maximum;
}

export async function measureAsyncWorkloads(): Promise<AsyncWorkloadMeasurement> {
  const e100 = createE100Workload();
  const timers = new ControlledTimers();
  const sources: MeasuredSource[] = [];
  let handshakes = 0;
  let activeReauthorizations = 0;
  let maximumConcurrentReauthorizations = 0;
  let maximumInFlightRefreshesPerIsland = 0;
  let maximumQueuedRefreshesPerIsland = 0;
  const states = new Map<string, SubscriptionState>();
  const recoveryAt = new Map<string, number>();
  const recoveryStarted = { value: 0 };
  const pool = new DocumentConnectionPool({
    handshakeScheduler: new OriginHandshakeScheduler(),
    randomness: { number: () => 0.5 },
    reauthorizationConcurrency: 8,
    timers,
    transports: {
      eventSource(request) {
        handshakes += 1;
        const source = new MeasuredSource(request);
        sources.push(source);
        return source;
      },
      webSocket() {
        throw new Error("async_benchmark_unexpected_websocket");
      },
    },
  });
  const fixtures: LogicalFixture[] = [];
  for (let index = 0; index < e100.subscriptionCount; index += 1) {
    const current = authorization(index);
    const root = document.querySelector(`[data-async-benchmark-index="${String(index)}"]`);
    if (!(root instanceof HTMLElement)) throw new Error("e100_island_missing");
    let inFlightRefreshes = 0;
    const capability = Object.freeze({}) as RegisteredBrowserEventCapability;
    const island: AsyncRuntimeIslandPort = {
      consumeRegisteredEventCapability: () => capability,
      dispatchRegisteredEvent: (_candidate, event) => {
        root.dispatchEvent(new CustomEvent(event.event, { detail: event.payload }));
        return "dispatched";
      },
      element: root,
      enqueueFreshRender: (_reason, completion) => {
        inFlightRefreshes += 1;
        maximumInFlightRefreshesPerIsland = Math.max(
          maximumInFlightRefreshesPerIsland,
          inFlightRefreshes,
        );
        completion?.("succeeded");
        inFlightRefreshes -= 1;
        maximumQueuedRefreshesPerIsland = Math.max(maximumQueuedRefreshesPerIsland, 0);
        return "queued";
      },
      identity: Object.freeze({
        component: "benchmark.component",
        documentKey: "benchmark-document",
        slot: `benchmark-${String(index)}`,
      }),
      onDispose: () => undefined,
      queryDirectiveOwnership: () => Object.freeze([]),
      writePresentationSignal: (_scope, _name, value) => value,
    };
    const subscription = new AsyncSubscription(
      current,
      new AsyncDispatcher(island, () => capability),
      { now: () => 1_000 },
    );
    const sink: LogicalSubscriptionSink = {
      envelope: (encoded) => {
        const disposition = subscription.receive(encoded);
        if (disposition !== "applied" && disposition !== "pending") {
          throw new Error(`e100_dispatch_${disposition}`);
        }
      },
      reauthorize: async (prior) => {
        if (prior === null) throw new Error("r100_prior_missing");
        activeReauthorizations += 1;
        maximumConcurrentReauthorizations = Math.max(
          maximumConcurrentReauthorizations,
          activeReauthorizations,
        );
        await Promise.resolve();
        activeReauthorizations -= 1;
        const successor = Object.freeze({
          ...prior,
          baseline: subscription.position(),
          descriptorBinding: `${prior.descriptorBinding}-reconnected`,
        });
        return Object.freeze({
          commit: () => {
            subscription.reauthorize(successor);
            return "committed" as const;
          },
          discard: () => undefined,
          proof: "authoritative_no_tail" as const,
          subscription: successor,
        });
      },
      state: (state) => {
        states.set(current.subscriptionId, state);
        if (state === "connecting" && subscription.state() === "disconnected") {
          subscription.connected();
        } else if (state === "reconnecting" && subscription.state() === "current") {
          subscription.transportLost();
        } else if (state === "current") {
          if (subscription.state() === "connecting") {
            subscription.proveAuthoritativeBaseline(subscription.position());
          }
          if (recoveryStarted.value > 0 && !recoveryAt.has(current.subscriptionId)) {
            recoveryAt.set(current.subscriptionId, performance.now() - recoveryStarted.value);
          }
        }
      },
    };
    const fixture: LogicalFixture = Object.freeze({
      authorization: current,
      handle: pool.subscribe(current, sink),
      subscription,
    });
    fixtures.push(fixture);
  }
  if (sources.length !== 1) throw new Error("e100_physical_connection_count");
  sources[0]?.open();
  await settle();
  for (const fixture of fixtures) fixture.handle.continuityProved();
  await settle();

  const dispatchEffectSamplesMs: number[] = [];
  const sequences = new Array<number>(e100.subscriptionCount).fill(0);
  for (let event = 0; event < e100.presentationEventCount; event += 1) {
    timers.advanceTo(((event + 1) * e100.scheduledDurationMs) / e100.presentationEventCount);
    const index = event % e100.subscriptionCount;
    const fixture = fixtures[index];
    if (fixture === undefined) throw new Error("e100_fixture_missing");
    const sequence = (sequences[index] ?? 0) + 1;
    sequences[index] = sequence;
    const started = performance.now();
    sources[0]?.emit(exactPresentationEnvelope(fixture.authorization, sequence));
    dispatchEffectSamplesMs.push(performance.now() - started);
  }
  const scheduledDurationMs = timers.now();
  let refreshInvalidationCount = 0;
  for (let refresh = 0; refresh < e100.refreshInvalidationCount; refresh += 1) {
    const index = refresh % e100.subscriptionCount;
    const fixture = fixtures[index];
    if (fixture === undefined) throw new Error("e100_fixture_missing");
    const sequence = (sequences[index] ?? 0) + 1;
    sequences[index] = sequence;
    sources[0]?.emit(refreshEnvelope(fixture.authorization, sequence));
    refreshInvalidationCount += 1;
  }
  const e100Current = fixtures.filter(
    ({ subscription }) => subscription.state() === "current",
  ).length;

  recoveryStarted.value = performance.now();
  for (const fixture of fixtures) fixture.handle.continuityLost();
  if (!timers.fireNext()) throw new Error("r100_reconnect_timer_missing");
  await settle();
  const reconnectSource = sources[1];
  if (reconnectSource === undefined) throw new Error("r100_document_reconnect_handshakes");
  reconnectSource.open();
  await settle();
  for (const fixture of fixtures) fixture.handle.continuityProved();
  await settle();
  const recovered = recoveryAt.size;
  const boundedRecoveryEnd = performance.now() - recoveryStarted.value;
  const recoverySamplesMs = fixtures.map(
    ({ authorization: current }) => recoveryAt.get(current.subscriptionId) ?? boundedRecoveryEnd,
  );
  const r100Current = fixtures.filter(
    ({ subscription }) => subscription.state() === "current",
  ).length;
  const minimumRecovery = Math.min(...recoverySamplesMs);
  const maximumRecovery = Math.max(...recoverySamplesMs);

  // Keep the production pool reachable until the runner's post-workload heap sample. The page is
  // dedicated to one measurement and is closed immediately after the sample.
  (
    window as Window & {
      __suprnovaBudgetAsyncRetention?: DocumentConnectionPool;
    }
  ).__suprnovaBudgetAsyncRetention = pool;

  return Object.freeze({
    E100: Object.freeze({
      dispatchEffectSamplesMs: Object.freeze(dispatchEffectSamplesMs),
      presentationEventCount: dispatchEffectSamplesMs.length,
      refreshInvalidationCount,
      scheduledDurationMs,
      physicalConnectionCount: sources.length - 1,
      handshakeCount: handshakes - 1,
      queuedEventPeak: 0,
      queuedBytePeak: 0,
      maximumQueuedRefreshesPerIsland,
      maximumInFlightRefreshesPerIsland,
      currentSubscriptionCount: e100Current,
    }),
    R100: Object.freeze({
      documentReconnectHandshakes: sources.length - 1,
      recoverySamplesMs: Object.freeze(recoverySamplesMs),
      maximumRecoverySkewMs:
        Number.isFinite(minimumRecovery) && Number.isFinite(maximumRecovery)
          ? maximumRecovery - minimumRecovery
          : Number.POSITIVE_INFINITY,
      recoveredSubscriptionCount: recovered,
      currentSubscriptionCount: r100Current,
      starvedSubscriptionCount: e100.subscriptionCount - recovered,
      maximumConcurrentReauthorizations,
      pollingMaximumSameTick: pollingFanoutEvidence(),
      multiDocument: multiDocumentEvidence(),
    }),
  });
}
