export type TransportFailureKind =
  | "http"
  | "media"
  | "size"
  | "protocol"
  | "correlation"
  | "network"
  | "offline"
  | "aborted"
  | "timeout"
  | "unsafe_endpoint";

export class LiveTransportError extends Error {
  constructor(
    readonly kind: TransportFailureKind,
    readonly status: number | null = null,
  ) {
    super(`live_transport_${kind}`);
    this.name = "LiveTransportError";
  }
}

export type TransportAttemptPhase = "created" | "fetching" | "reading" | "settled";

export class TransportAttemptState {
  #phase: TransportAttemptPhase = "created";
  #timedOut = false;
  #userAborted = false;

  phase(): TransportAttemptPhase {
    return this.#phase;
  }

  beginFetch(): void {
    if (this.#phase !== "created") throw new LiveTransportError("network");
    this.#phase = "fetching";
  }

  beginRead(): void {
    if (this.#phase !== "fetching") throw new LiveTransportError("network");
    this.#phase = "reading";
  }

  timeout(): void {
    if (this.#phase !== "settled") this.#timedOut = true;
  }

  userAbort(): void {
    if (this.#phase !== "settled") this.#userAborted = true;
  }

  settle(): void {
    this.#phase = "settled";
  }

  interruption(isOnline: () => boolean): LiveTransportError {
    if (this.#timedOut) return new LiveTransportError("timeout");
    if (this.#userAborted) return new LiveTransportError("aborted");
    try {
      if (!isOnline()) return new LiveTransportError("offline");
    } catch {
      // A hostile connectivity observer cannot expose detail or make a retry safe.
    }
    return new LiveTransportError("network");
  }
}

export interface LiveTransportResponse {
  readonly protocolVersion: 1 | 2;
  readonly status: number;
  readonly text: string;
}
