import { ContinuityMachine } from "./continuity.js";
import { decodeAsyncEnvelope, validExpiration } from "./envelope.js";
import type {
  AsyncClock,
  AsyncDispatchPort,
  AsyncEnvelope,
  AsyncReceiveDisposition,
  AuthorizedLogicalSubscription,
  StreamPosition,
  SubscriptionState,
} from "./types.js";

const MAX_REPLAY_BYTES = 256 * 1024;

export interface ReplayOutcome {
  readonly applied: number;
  readonly through: StreamPosition;
}

export type AsyncContinuityProof = "authoritative_no_tail" | "complete_replay";

export class AsyncSubscription {
  #authorization: AuthorizedLogicalSubscription;
  readonly #clock: AsyncClock;
  #continuity: ContinuityMachine;
  readonly #dispatch: AsyncDispatchPort;

  constructor(
    authorization: AuthorizedLogicalSubscription,
    dispatch: AsyncDispatchPort,
    clock: AsyncClock,
  ) {
    if (!validExpiration(authorization.expiresAt) || typeof clock.now !== "function") {
      throw new Error("async_subscription_invalid");
    }
    this.#authorization = authorization;
    this.#continuity = new ContinuityMachine(authorization.baseline);
    this.#dispatch = dispatch;
    this.#clock = clock;
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
    if (!this.#dispatchEnvelope(envelope)) {
      this.#continuity.degrade();
      return "dispatch_failed";
    }
    this.#continuity.commit(envelope.position);
    if (envelope.payload.kind === "complete") this.#continuity.close();
    else if (envelope.payload.kind === "error") this.#continuity.degrade();
    return "applied";
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
      if (!this.#dispatchEnvelope(envelope)) {
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

  #dispatchEnvelope(envelope: AsyncEnvelope): boolean {
    switch (envelope.payload.kind) {
      case "refresh":
        return this.#dispatch.refresh(envelope.payload);
      case "browser_event":
        return this.#dispatch.browserEvent(envelope.payload);
      case "presentation_signal":
        return this.#dispatch.presentationSignal(envelope.payload);
      case "heartbeat":
      case "complete":
      case "error":
        return true;
    }
  }
}
