import { ContinuityMachine } from "./continuity.js";
import type { AsyncEnvelopeDispatcher } from "./dispatch.js";
import type { PartiallyDispatchedBrowserEvent } from "../features/contract.js";
import {
  comparePosition,
  decodeAsyncEnvelope,
  isExactSuccessor,
  validExpiration,
} from "./envelope.js";
import type {
  AsyncClock,
  AsyncReceiveDisposition,
  AuthorizedLogicalSubscription,
  StreamPosition,
  SubscriptionState,
  ValidatedAsyncEnvelope,
} from "./types.js";
import type { FreshRenderCompletion } from "../features/contract.js";

const MAX_REPLAY_BYTES = 256 * 1024;

export interface ReplayOutcome {
  readonly applied: number;
  readonly through: StreamPosition;
}

type ReplayDisposition = ReplayOutcome | "pending";

interface RefreshSegment {
  count: number;
  encodedBytes: number;
  readonly first: StreamPosition;
  readonly generation: number;
  readonly kind: "refresh";
  last: StreamPosition;
  envelope: ValidatedAsyncEnvelope;
}

type PendingPresentation =
  | Readonly<{ encodedBytes: number; envelope: ValidatedAsyncEnvelope; kind: "envelope" }>
  | (RefreshSegment & Readonly<{ kind: "refresh" }>);

const MAX_PENDING_PRESENTATIONS = 1_024;

export interface AsyncQueuePressureObservation {
  readonly inFlightRefreshes: number;
  readonly queuedBytes: number;
  readonly queuedEvents: number;
  readonly queuedRefreshes: number;
}

export type AsyncQueuePressureObserver = (observation: AsyncQueuePressureObservation) => void;

export type AsyncContinuityProof = "authoritative_no_tail" | "complete_replay";

export type AsyncSubscriptionLifecycleOutcome =
  | Readonly<{
      kind: "complete";
      reason: "server_shutdown" | "subscription_retired" | "stream_completed";
    }>
  | Readonly<{
      kind: "error";
      reason: "authorization_lost" | "replay_unavailable" | "backpressure" | "stream_unavailable";
    }>
  | Readonly<{
      kind: "dispatch_failed";
      reason:
        | "presentation_rejected"
        | "refresh_failed"
        | "refresh_canceled"
        | "refresh_retired"
        | "resource_exhausted";
    }>
  | Readonly<{
      delivered: number;
      detail: PartiallyDispatchedBrowserEvent["reason"];
      kind: "dispatch_failed";
      reason: "presentation_partial";
      skipped: number;
    }>;

export class AsyncSubscription {
  #authorization: AuthorizedLogicalSubscription;
  readonly #clock: AsyncClock;
  #continuity: ContinuityMachine;
  readonly #dispatch: AsyncEnvelopeDispatcher;
  readonly #refreshCompletion: ((completion: FreshRenderCompletion) => void) | undefined;
  readonly #lifecycle: ((outcome: AsyncSubscriptionLifecycleOutcome) => void) | undefined;
  readonly #queueObserver: AsyncQueuePressureObserver | undefined;
  readonly #pending: PendingPresentation[] = [];
  #pendingBytes = 0;
  #activeRefresh: RefreshSegment | null = null;
  #observedPosition: StreamPosition;
  #pendingAdmissions = 0;
  #replayActive = false;
  #lifecycleGeneration = 0;

  constructor(
    authorization: AuthorizedLogicalSubscription,
    dispatch: AsyncEnvelopeDispatcher,
    clock: AsyncClock,
    refreshCompletion?: (completion: FreshRenderCompletion) => void,
    lifecycle?: (outcome: AsyncSubscriptionLifecycleOutcome) => void,
    queueObserver?: AsyncQueuePressureObserver,
  ) {
    if (!validExpiration(authorization.expiresAt) || typeof clock.now !== "function") {
      throw new Error("async_subscription_invalid");
    }
    this.#authorization = authorization;
    this.#continuity = new ContinuityMachine(authorization.baseline);
    this.#observedPosition = authorization.baseline;
    this.#dispatch = dispatch;
    this.#clock = clock;
    this.#refreshCompletion = refreshCompletion;
    this.#lifecycle = lifecycle;
    this.#queueObserver = queueObserver;
  }

  state(): SubscriptionState {
    return this.#continuity.state();
  }

  position(): StreamPosition {
    return this.#continuity.position();
  }

  connected(): void {
    this.#continuity.connected();
  }

  transportLost(): void {
    this.#lifecycleGeneration += 1;
    this.#continuity.transportLost();
  }

  heartbeatLost(): void {
    this.#lifecycleGeneration += 1;
    this.#continuity.degrade();
  }

  authorizationUncertain(): void {
    this.#lifecycleGeneration += 1;
    this.#continuity.degrade();
  }

  close(): void {
    this.#lifecycleGeneration += 1;
    this.#clearPending();
    this.#continuity.close();
  }

  reauthorize(authorization: AuthorizedLogicalSubscription): void {
    const position = this.#continuity.position();
    if (
      !validExpiration(authorization.expiresAt) ||
      authorization.subscriptionId !== this.#authorization.subscriptionId ||
      authorization.stream !== this.#authorization.stream ||
      ((authorization.baseline.epoch !== position.epoch ||
        authorization.baseline.sequence !== position.sequence) &&
        !this.#continuity.acceptsAuthoritativeBaseline(authorization.baseline))
    ) {
      this.#continuity.degrade();
      throw new Error("async_reauthorization_invalid");
    }
    this.#lifecycleGeneration += 1;
    this.#authorization = authorization;
    this.#observedPosition = position;
    this.#continuity.transportLost();
    this.#continuity.connected();
  }

  preflightReauthorization(
    authorization: AuthorizedLogicalSubscription,
    encoded: readonly string[],
  ): AsyncContinuityProof {
    const position = this.#continuity.position();
    if (
      !validExpiration(authorization.expiresAt) ||
      authorization.subscriptionId !== this.#authorization.subscriptionId ||
      authorization.stream !== this.#authorization.stream ||
      ((authorization.baseline.epoch !== position.epoch ||
        authorization.baseline.sequence !== position.sequence) &&
        encoded.length !== 0)
    ) {
      throw new Error("async_reauthorization_invalid");
    }
    return this.#preflightReplay(encoded, authorization);
  }

  preflightInitialReplay(encoded: readonly string[]): AsyncContinuityProof {
    return this.#preflightReplay(encoded, this.#authorization);
  }

  preflightFreshInitialReplay(
    authorization: AuthorizedLogicalSubscription,
    encoded: readonly string[],
  ): AsyncContinuityProof {
    this.#assertSameLogicalMembership(authorization);
    return this.#preflightReplay(
      encoded,
      authorization,
      new ContinuityMachine(authorization.baseline),
    );
  }

  replaceUncommittedInitial(authorization: AuthorizedLogicalSubscription): void {
    this.#assertSameLogicalMembership(authorization);
    if (!validExpiration(authorization.expiresAt)) throw new Error("async_subscription_invalid");
    this.#authorization = authorization;
    this.#lifecycleGeneration += 1;
    this.#continuity = new ContinuityMachine(authorization.baseline);
    this.#observedPosition = authorization.baseline;
    this.#clearPending();
    this.#continuity.connected();
  }

  receive(encoded: string): AsyncReceiveDisposition {
    this.#assertCurrentAuthority();
    const envelope = decodeAsyncEnvelope(encoded, this.#authorization);
    if (this.#activeRefresh !== null) {
      return this.#enqueueWhileRefreshPending(
        envelope,
        new TextEncoder().encode(encoded).byteLength,
      );
    }
    const observation = this.#continuity.observe(envelope.position);
    if (observation !== "apply") return observation;
    this.#observedPosition = envelope.position;
    return this.#applyEnvelope(envelope);
  }

  #completeRefresh(completion: FreshRenderCompletion): void {
    const active = this.#activeRefresh;
    if (active === null) return;
    this.#activeRefresh = null;
    this.#observeQueuePressure();
    const terminal = active.generation === this.#lifecycleGeneration ? completion : "canceled";
    let notifyRecovery = terminal !== "succeeded";
    if (terminal === "succeeded") {
      this.#commitRange(active);
      this.#drainPending();
      notifyRecovery = this.#presentationSettled();
    } else {
      this.#failPresentation(
        terminal === "failed"
          ? "refresh_failed"
          : terminal === "canceled"
            ? "refresh_canceled"
            : "refresh_retired",
        active.last,
      );
    }
    if (notifyRecovery) {
      try {
        this.#refreshCompletion?.(terminal);
      } catch {
        // Presentation/recovery observation cannot rewrite continuity authority.
      }
    }
  }

  #applyEnvelope(envelope: ValidatedAsyncEnvelope): AsyncReceiveDisposition {
    if (envelope.payload.kind === "refresh") {
      this.#startRefresh({
        count: 1,
        encodedBytes: 0,
        envelope,
        first: envelope.position,
        generation: this.#lifecycleGeneration,
        kind: "refresh",
        last: envelope.position,
      });
      return this.#activeRefresh === null && this.#continuity.state() === "degraded"
        ? "dispatch_failed"
        : "pending";
    }
    const disposition = this.#dispatch.dispatch(envelope);
    if (disposition === "rejected" || typeof disposition === "object") {
      this.#failPresentation(
        typeof disposition === "object" ? "presentation_partial" : "presentation_rejected",
        envelope.position,
        typeof disposition === "object" ? disposition : undefined,
      );
      return "dispatch_failed";
    }
    this.#continuity.commit(envelope.position);
    if (envelope.payload.kind === "complete") {
      this.#clearPending();
      this.#continuity.close();
      this.#observeLifecycle(Object.freeze({ kind: "complete", reason: envelope.payload.reason }));
    } else if (envelope.payload.kind === "error") {
      this.#continuity.degradeAt(envelope.position);
      this.#observeLifecycle(Object.freeze({ kind: "error", reason: envelope.payload.code }));
    }
    return "applied";
  }

  #startRefresh(segment: RefreshSegment): void {
    this.#activeRefresh = segment;
    this.#observeQueuePressure();
    const synchronousTerminal: { value: FreshRenderCompletion | null } = { value: null };
    let dispatching = true;
    const disposition = this.#dispatch.dispatch(segment.envelope, (completion) => {
      if (dispatching) synchronousTerminal.value = completion;
      else this.#completeRefresh(completion);
    });
    dispatching = false;
    if (disposition === "exhausted") {
      this.#activeRefresh = null;
      this.#observeQueuePressure();
      this.#failPresentation("resource_exhausted", segment.last);
      return;
    }
    if (disposition !== "queued" && disposition !== "coalesced") {
      this.#activeRefresh = null;
      this.#observeQueuePressure();
      this.#failPresentation("presentation_rejected", segment.last);
      return;
    }
    if (synchronousTerminal.value !== null) this.#completeRefresh(synchronousTerminal.value);
  }

  #enqueueWhileRefreshPending(
    envelope: ValidatedAsyncEnvelope,
    encodedBytes: number,
  ): AsyncReceiveDisposition {
    const ordering = comparePosition(envelope.position, this.#observedPosition);
    if (ordering === 0) return "duplicate";
    if (ordering < 0 || envelope.position.epoch < this.#observedPosition.epoch) return "stale";
    if (!isExactSuccessor(this.#observedPosition, envelope.position)) {
      this.#observedPosition = envelope.position;
      this.#failPresentation("presentation_rejected", envelope.position);
      return "gap";
    }
    this.#observedPosition = envelope.position;
    this.#pendingAdmissions += 1;
    if (this.#pendingAdmissions > MAX_PENDING_PRESENTATIONS) {
      this.#failPresentation("presentation_rejected", envelope.position);
      return "dispatch_failed";
    }
    const tail = this.#pending[this.#pending.length - 1];
    if (envelope.payload.kind === "refresh" && tail?.kind === "refresh") {
      tail.count += 1;
      tail.encodedBytes += encodedBytes;
      tail.envelope = envelope;
      tail.last = envelope.position;
    } else if (envelope.payload.kind === "refresh") {
      this.#pending.push({
        count: 1,
        encodedBytes,
        envelope,
        first: envelope.position,
        generation: this.#lifecycleGeneration,
        kind: "refresh",
        last: envelope.position,
      });
    } else {
      this.#pending.push(Object.freeze({ encodedBytes, envelope, kind: "envelope" }));
    }
    this.#pendingBytes += encodedBytes;
    this.#observeQueuePressure();
    return "pending";
  }

  #drainPending(): "complete" | "dispatch_failed" | "interrupted" | "pending" {
    while (this.#pending.length !== 0 && this.#continuity.state() !== "closed") {
      const next = this.#pending.shift();
      if (next === undefined) break;
      this.#pendingBytes -= next.encodedBytes;
      this.#pendingAdmissions -= next.kind === "refresh" ? next.count : 1;
      this.#observeQueuePressure();
      if (next.kind === "refresh") {
        this.#startRefresh(next);
        return this.#activeRefresh === null
          ? this.#continuity.state() === "degraded"
            ? "dispatch_failed"
            : "complete"
          : "pending";
      }
      if (this.#applyEnvelope(next.envelope) !== "applied") return "dispatch_failed";
      if (this.#continuity.state() === "degraded") {
        this.#pending.length = 0;
        this.#pendingAdmissions = 0;
        this.#replayActive = false;
        return "interrupted";
      }
    }
    if (this.#replayActive) {
      this.#replayActive = false;
      this.#continuity.finishReplay();
    }
    return "complete";
  }

  #presentationSettled(): boolean {
    return this.#activeRefresh === null && this.#pending.length === 0 && !this.#replayActive;
  }

  #commitRange(segment: RefreshSegment): void {
    let position = segment.first;
    for (let index = 0; index < segment.count; index += 1) {
      this.#continuity.commit(position);
      position = Object.freeze({ epoch: position.epoch, sequence: position.sequence + 1n });
    }
  }

  #failPresentation(
    reason: Extract<AsyncSubscriptionLifecycleOutcome, { kind: "dispatch_failed" }>["reason"],
    candidate: StreamPosition,
    partial?: PartiallyDispatchedBrowserEvent,
  ): void {
    const highWater =
      comparePosition(this.#observedPosition, candidate) > 0 ? this.#observedPosition : candidate;
    if (reason === "presentation_partial") this.#continuity.degradeNonReplayableAt(highWater);
    else this.#continuity.degradeAt(highWater);
    this.#clearPending();
    this.#observeLifecycle(
      reason === "presentation_partial" && partial !== undefined
        ? Object.freeze({
            delivered: partial.delivered,
            detail: partial.reason,
            kind: "dispatch_failed",
            reason,
            skipped: partial.skipped,
          })
        : (Object.freeze({ kind: "dispatch_failed", reason }) as AsyncSubscriptionLifecycleOutcome),
    );
  }

  #clearPending(): void {
    this.#activeRefresh = null;
    this.#pending.length = 0;
    this.#pendingAdmissions = 0;
    this.#pendingBytes = 0;
    this.#replayActive = false;
    this.#observeQueuePressure();
  }

  #observeLifecycle(outcome: AsyncSubscriptionLifecycleOutcome): void {
    try {
      this.#lifecycle?.(outcome);
    } catch {
      // Lifecycle presentation cannot rewrite continuity authority.
    }
  }

  receiveReplay(encoded: readonly string[]): ReplayDisposition {
    this.#assertCurrentAuthority();
    if (encoded.length === 0 || encoded.length > 1_024) throw new Error("async_replay_invalid");
    let replayBytes = 0;
    for (const value of encoded) {
      replayBytes += new TextEncoder().encode(value).byteLength;
      if (replayBytes > MAX_REPLAY_BYTES) throw new Error("async_replay_too_large");
    }
    const transcript = encoded.map((value) => decodeAsyncEnvelope(value, this.#authorization));
    if (transcript.some(({ payload }) => payload.kind === "complete")) {
      throw new Error("async_replay_invalid");
    }
    this.#continuity.validateReplay(transcript.map(({ position }) => position));
    if (this.#activeRefresh !== null || this.#pending.length !== 0) {
      throw new Error("async_replay_pending");
    }
    this.#replayActive = true;
    for (const [index, envelope] of transcript.entries()) {
      this.#assertCurrentAuthority();
      this.#observedPosition = envelope.position;
      this.#pendingAdmissions += 1;
      const value = encoded[index];
      if (value === undefined) throw new Error("async_replay_invalid");
      const encodedBytes = new TextEncoder().encode(value).byteLength;
      const tail = this.#pending[this.#pending.length - 1];
      if (envelope.payload.kind === "refresh" && tail?.kind === "refresh") {
        tail.count += 1;
        tail.encodedBytes += encodedBytes;
        tail.envelope = envelope;
        tail.last = envelope.position;
      } else if (envelope.payload.kind === "refresh") {
        this.#pending.push({
          count: 1,
          encodedBytes,
          envelope,
          first: envelope.position,
          generation: this.#lifecycleGeneration,
          kind: "refresh",
          last: envelope.position,
        });
      } else {
        this.#pending.push(Object.freeze({ encodedBytes, envelope, kind: "envelope" }));
      }
      this.#pendingBytes += encodedBytes;
      this.#observeQueuePressure();
    }
    const drain = this.#drainPending();
    if (drain === "interrupted") {
      throw new Error("async_replay_interrupted");
    }
    if (drain === "dispatch_failed") {
      throw new Error("async_replay_dispatch_failed");
    }
    if (drain === "pending") return "pending";
    return Object.freeze({ applied: transcript.length, through: this.#continuity.position() });
  }

  #preflightReplay(
    encoded: readonly string[],
    authorization: AuthorizedLogicalSubscription,
    continuity = this.#continuity,
  ): AsyncContinuityProof {
    const now = this.#clock.now();
    if (!Number.isSafeInteger(now) || now < 0 || now >= authorization.expiresAt) {
      throw new Error("async_membership_expired");
    }
    if (encoded.length === 0) {
      continuity.validateAuthoritativeBaseline(authorization.baseline);
      return "authoritative_no_tail";
    }
    if (encoded.length > 1_024) throw new Error("async_replay_invalid");
    let replayBytes = 0;
    for (const value of encoded) {
      replayBytes += new TextEncoder().encode(value).byteLength;
      if (replayBytes > MAX_REPLAY_BYTES) throw new Error("async_replay_too_large");
    }
    const transcript = encoded.map((value) => decodeAsyncEnvelope(value, authorization));
    if (transcript.some(({ payload }) => payload.kind === "complete")) {
      throw new Error("async_replay_invalid");
    }
    continuity.validateReplay(transcript.map(({ position }) => position));
    return "complete_replay";
  }

  #assertSameLogicalMembership(authorization: AuthorizedLogicalSubscription): void {
    if (
      authorization.subscriptionId !== this.#authorization.subscriptionId ||
      authorization.stream !== this.#authorization.stream
    ) {
      throw new Error("async_reauthorization_invalid");
    }
  }

  proveAuthoritativeBaseline(position: StreamPosition): void {
    this.#assertCurrentAuthority();
    this.#continuity.proveAuthoritativeBaseline(position);
  }

  #assertCurrentAuthority(): void {
    const now = this.#clock.now();
    if (!Number.isSafeInteger(now) || now < 0 || now >= this.#authorization.expiresAt) {
      this.#continuity.degrade();
      throw new Error("async_membership_expired");
    }
  }

  #observeQueuePressure(): void {
    const observer = this.#queueObserver;
    if (observer === undefined) return;
    try {
      observer(
        Object.freeze({
          inFlightRefreshes: this.#activeRefresh === null ? 0 : 1,
          queuedBytes: this.#pendingBytes,
          queuedEvents: this.#pendingAdmissions,
          queuedRefreshes: this.#pending.some(({ kind }) => kind === "refresh") ? 1 : 0,
        }),
      );
    } catch {
      // Count-only observation cannot rewrite presentation or continuity authority.
    }
  }
}
