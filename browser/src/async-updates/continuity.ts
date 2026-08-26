import { comparePosition, isExactSuccessor } from "./envelope.js";
import type { StreamPosition, SubscriptionState } from "./types.js";

export type SequenceObservation =
  "apply" | "duplicate" | "stale" | "gap" | "continuity_required" | "closed";

function copy(position: StreamPosition): StreamPosition {
  return Object.freeze({ epoch: position.epoch, sequence: position.sequence });
}

export class ContinuityMachine {
  #position: StreamPosition;
  #state: SubscriptionState = "disconnected";
  #requiredHighWater: StreamPosition | null = null;
  #proofRequired = false;

  constructor(baseline: StreamPosition) {
    this.#position = copy(baseline);
  }

  state(): SubscriptionState {
    return this.#state;
  }

  position(): StreamPosition {
    return copy(this.#position);
  }

  connected(): void {
    if (this.#state !== "closed") this.#state = "connecting";
  }

  transportLost(): void {
    if (this.#state !== "closed") {
      this.#proofRequired = true;
      this.#state = "reconnecting";
    }
  }

  degrade(): void {
    if (this.#state !== "closed") {
      this.#proofRequired = true;
      this.#state = "degraded";
    }
  }

  close(): void {
    this.#state = "closed";
  }

  observe(candidate: StreamPosition): SequenceObservation {
    if (this.#state === "closed") return "closed";
    const ordering = comparePosition(candidate, this.#position);
    if (ordering === 0) return "duplicate";
    if (ordering < 0 || candidate.epoch < this.#position.epoch) return "stale";
    if (this.#proofRequired) {
      this.#recordHighWater(candidate);
      return "continuity_required";
    }
    if (isExactSuccessor(this.#position, candidate)) return "apply";
    this.#recordHighWater(candidate);
    this.#state = "degraded";
    return "gap";
  }

  commit(candidate: StreamPosition): void {
    if (!isExactSuccessor(this.#position, candidate))
      throw new Error("async_sequence_commit_invalid");
    this.#position = copy(candidate);
    this.#state = "current";
  }

  validateReplay(positions: readonly StreamPosition[]): void {
    if (this.#state === "closed" || positions.length === 0 || positions.length > 1_024) {
      throw new Error("async_replay_invalid");
    }
    let prior = this.#position;
    for (const position of positions) {
      if (!isExactSuccessor(prior, position)) throw new Error("async_replay_invalid");
      prior = position;
    }
    if (this.#requiredHighWater !== null && comparePosition(prior, this.#requiredHighWater) < 0) {
      throw new Error("async_replay_incomplete");
    }
  }

  finishReplay(): void {
    if (this.#state === "closed") throw new Error("async_replay_invalid");
    this.#requiredHighWater = null;
    this.#proofRequired = false;
    this.#state = "current";
  }

  #recordHighWater(candidate: StreamPosition): void {
    if (
      this.#requiredHighWater === null ||
      comparePosition(candidate, this.#requiredHighWater) > 0
    ) {
      this.#requiredHighWater = copy(candidate);
    }
  }
}
