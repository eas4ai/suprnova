import { ContinuityMachine } from "./continuity.js";
import type { AsyncEnvelopeDispatcher } from "./dispatch.js";
import { decodeAsyncEnvelope, validExpiration } from "./envelope.js";
import type {
  AsyncClock,
  AsyncReceiveDisposition,
  AuthorizedLogicalSubscription,
  StreamPosition,
  SubscriptionState,
} from "./types.js";
import type { FreshRenderCompletion } from "../features/contract.js";

const MAX_REPLAY_BYTES = 256 * 1024;

export interface ReplayOutcome {
  readonly applied: number;
  readonly through: StreamPosition;
}

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
      reason: "presentation_rejected" | "refresh_failed" | "refresh_canceled" | "refresh_retired";
    }>;

export class AsyncSubscription {
  #authorization: AuthorizedLogicalSubscription;
  readonly #clock: AsyncClock;
  #continuity: ContinuityMachine;
  readonly #dispatch: AsyncEnvelopeDispatcher;
  readonly #refreshCompletion: ((completion: FreshRenderCompletion) => void) | undefined;
  readonly #lifecycle: ((outcome: AsyncSubscriptionLifecycleOutcome) => void) | undefined;

  constructor(
    authorization: AuthorizedLogicalSubscription,
    dispatch: AsyncEnvelopeDispatcher,
    clock: AsyncClock,
    refreshCompletion?: (completion: FreshRenderCompletion) => void,
    lifecycle?: (outcome: AsyncSubscriptionLifecycleOutcome) => void,
  ) {
    if (!validExpiration(authorization.expiresAt) || typeof clock.now !== "function") {
      throw new Error("async_subscription_invalid");
    }
    this.#authorization = authorization;
    this.#continuity = new ContinuityMachine(authorization.baseline);
    this.#dispatch = dispatch;
    this.#clock = clock;
    this.#refreshCompletion = refreshCompletion;
    this.#lifecycle = lifecycle;
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
    this.#continuity.transportLost();
  }

  heartbeatLost(): void {
    this.#continuity.degrade();
  }

  authorizationUncertain(): void {
    this.#continuity.degrade();
  }

  close(): void {
    this.#continuity.close();
  }

  reauthorize(authorization: AuthorizedLogicalSubscription): void {
    const position = this.#continuity.position();
    if (
      !validExpiration(authorization.expiresAt) ||
      authorization.subscriptionId !== this.#authorization.subscriptionId ||
      authorization.stream !== this.#authorization.stream ||
      authorization.baseline.epoch !== position.epoch ||
      authorization.baseline.sequence !== position.sequence
    ) {
      this.#continuity.degrade();
      throw new Error("async_reauthorization_invalid");
    }
    this.#authorization = authorization;
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
      authorization.baseline.epoch !== position.epoch ||
      authorization.baseline.sequence !== position.sequence
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
    this.#continuity = new ContinuityMachine(authorization.baseline);
    this.#continuity.connected();
  }

  receive(encoded: string): AsyncReceiveDisposition {
    this.#assertCurrentAuthority();
    const envelope = decodeAsyncEnvelope(encoded, this.#authorization);
    const observation = this.#continuity.observe(envelope.position);
    if (observation !== "apply") return observation;
    const terminal: { value: FreshRenderCompletion | null } = { value: null };
    let committed = false;
    const dispatchDisposition = this.#dispatch.dispatch(envelope, (completion) => {
      if (!committed) {
        terminal.value = completion;
        return;
      }
      this.#completeRefresh(completion);
    });
    if (dispatchDisposition === "rejected") {
      this.#continuity.degrade();
      this.#observeLifecycle(
        Object.freeze({ kind: "dispatch_failed", reason: "presentation_rejected" }),
      );
      return "dispatch_failed";
    }
    this.#continuity.commit(envelope.position);
    committed = true;
    if (envelope.payload.kind === "refresh") {
      if (terminal.value !== null) this.#completeRefresh(terminal.value);
      return "pending";
    }
    if (envelope.payload.kind === "complete") {
      this.#continuity.close();
      this.#observeLifecycle(Object.freeze({ kind: "complete", reason: envelope.payload.reason }));
    } else if (envelope.payload.kind === "error") {
      this.#continuity.degrade();
      this.#observeLifecycle(Object.freeze({ kind: "error", reason: envelope.payload.code }));
    }
    return "applied";
  }

  #completeRefresh(completion: FreshRenderCompletion): void {
    if (completion !== "succeeded") {
      this.#continuity.degrade();
      this.#observeLifecycle(
        Object.freeze({
          kind: "dispatch_failed",
          reason:
            completion === "failed"
              ? "refresh_failed"
              : completion === "canceled"
                ? "refresh_canceled"
                : "refresh_retired",
        }),
      );
    }
    try {
      this.#refreshCompletion?.(completion);
    } catch {
      // Presentation/recovery observation cannot rewrite continuity authority.
    }
  }

  #observeLifecycle(outcome: AsyncSubscriptionLifecycleOutcome): void {
    try {
      this.#lifecycle?.(outcome);
    } catch {
      // Lifecycle presentation cannot rewrite continuity authority.
    }
  }

  receiveReplay(encoded: readonly string[]): ReplayOutcome {
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
    let applied = 0;
    for (const envelope of transcript) {
      this.#assertCurrentAuthority();
      if (this.#dispatch.dispatch(envelope) === "rejected") {
        this.#continuity.degrade();
        throw new Error("async_replay_dispatch_failed");
      }
      this.#continuity.commit(envelope.position);
      applied += 1;
      if (envelope.payload.kind === "error") {
        this.#continuity.degrade();
        throw new Error("async_replay_interrupted");
      }
    }
    this.#continuity.finishReplay();
    return Object.freeze({ applied, through: this.#continuity.position() });
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
}
