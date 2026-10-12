export type RecoveryState = "none" | "fresh_render_pending" | "disconnected";

export interface RecoveryIdentity {
  readonly acceptedRevision: bigint;
  readonly connectionEpoch: number;
}

export interface ApplicationEpoch extends RecoveryIdentity {
  readonly epoch: number;
}

export type RecoveryDisposition = "request_fresh_render" | "disconnect_island" | "ignored";

export interface RecoveryDecision {
  readonly disposition: RecoveryDisposition;
}

export interface FreshRenderOperation {
  readonly kind: "fresh_render";
  readonly modelProposals: readonly never[];
  readonly childParameters: readonly never[];
  readonly originalAction: null;
}

const FRESH_RENDER_OPERATION: FreshRenderOperation = Object.freeze({
  childParameters: Object.freeze([]),
  kind: "fresh_render",
  modelProposals: Object.freeze([]),
  originalAction: null,
});

function validIdentity(identity: RecoveryIdentity): boolean {
  return (
    identity.acceptedRevision >= 0n &&
    Number.isSafeInteger(identity.connectionEpoch) &&
    identity.connectionEpoch >= 0
  );
}

export class ApplicationRecovery {
  #state: RecoveryState = "none";
  #connectionEpoch: number | null = null;
  #epoch = 0;
  #current: ApplicationEpoch | null = null;

  state(): RecoveryState {
    return this.#state;
  }

  freshRenderOperation(): FreshRenderOperation {
    return FRESH_RENDER_OPERATION;
  }

  begin(identity: RecoveryIdentity): ApplicationEpoch | null {
    if (!validIdentity(identity)) throw new Error("recovery_identity_invalid");
    if (this.#connectionEpoch !== null && this.#connectionEpoch !== identity.connectionEpoch) {
      this.#state = "none";
      this.#current = null;
    }
    this.#connectionEpoch = identity.connectionEpoch;
    if (this.#state === "disconnected") return null;
    if (this.#epoch >= Number.MAX_SAFE_INTEGER) {
      this.#state = "disconnected";
      this.#current = null;
      return null;
    }
    this.#epoch += 1;
    const token = Object.freeze({ ...identity, epoch: this.#epoch });
    this.#current = token;
    return token;
  }

  current(token: ApplicationEpoch | null): boolean {
    return token !== null && this.#current === token && this.#state !== "disconnected";
  }

  fail(token: ApplicationEpoch | null): RecoveryDecision {
    if (!this.current(token)) return Object.freeze({ disposition: "ignored" });
    this.#current = null;
    if (this.#state === "fresh_render_pending") {
      this.#state = "disconnected";
      return Object.freeze({ disposition: "disconnect_island" });
    }
    this.#state = "fresh_render_pending";
    return Object.freeze({ disposition: "request_fresh_render" });
  }

  succeed(token: ApplicationEpoch | null): boolean {
    if (!this.current(token)) return false;
    this.#current = null;
    this.#state = "none";
    return true;
  }

  disconnect(): void {
    this.#current = null;
    this.#state = "disconnected";
  }
}
