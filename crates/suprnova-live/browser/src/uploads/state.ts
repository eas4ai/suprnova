export type UploadProtocolState =
  | "canceled"
  | "created"
  | "expired"
  | "failed"
  | "finalized"
  | "finalizing"
  | "queued"
  | "ready"
  | "rejected"
  | "transferring"
  | "verifying";

export type UploadProtocolTransition =
  | "accept"
  | "begin_finalize"
  | "begin_transfer"
  | "cancel"
  | "commit_finalize"
  | "complete"
  | "expire"
  | "fail"
  | "put_chunk"
  | "queue"
  | "reject";

export interface UploadProtocolTransitionRequest {
  readonly expectedRevision: bigint;
  readonly idempotencyKey: string;
  readonly transition: UploadProtocolTransition;
}

export interface UploadProtocolTransitionOutcome {
  readonly disposition: "applied" | "existing_outcome";
  readonly revision: bigint;
  readonly state: UploadProtocolState;
}

export class UploadProtocolStateError extends Error {
  constructor(
    readonly code:
      | "invalid_upload_transition"
      | "revision_exhausted"
      | "upload_conflict"
      | "upload_idempotency_history_full",
  ) {
    super(code);
    this.name = "UploadProtocolStateError";
  }
}

const MAX_U64 = 18_446_744_073_709_551_615n;
const MAX_RETAINED_OUTCOMES = 64;

export function parseUploadProtocolState(value: unknown): UploadProtocolState {
  switch (value) {
    case "canceled":
    case "created":
    case "expired":
    case "failed":
    case "finalized":
    case "finalizing":
    case "queued":
    case "ready":
    case "rejected":
    case "transferring":
    case "verifying":
      return value;
    default:
      throw new UploadProtocolStateError("invalid_upload_transition");
  }
}

export function parseUploadProtocolTransition(value: unknown): UploadProtocolTransition {
  switch (value) {
    case "accept":
    case "begin_finalize":
    case "begin_transfer":
    case "cancel":
    case "commit_finalize":
    case "complete":
    case "expire":
    case "fail":
    case "put_chunk":
    case "queue":
    case "reject":
      return value;
    default:
      throw new UploadProtocolStateError("invalid_upload_transition");
  }
}

export function isTerminalUploadProtocolState(state: UploadProtocolState): boolean {
  return (
    state === "canceled" ||
    state === "expired" ||
    state === "failed" ||
    state === "finalized" ||
    state === "rejected"
  );
}

function nextState(
  state: UploadProtocolState,
  transition: UploadProtocolTransition,
): UploadProtocolState {
  if (isTerminalUploadProtocolState(state)) {
    throw new UploadProtocolStateError("invalid_upload_transition");
  }
  switch (transition) {
    case "queue":
      if (state === "created") return "queued";
      break;
    case "begin_transfer":
      if (state === "queued") return "transferring";
      break;
    case "put_chunk":
      if (state === "transferring") return "transferring";
      break;
    case "complete":
      if (state === "transferring") return "verifying";
      break;
    case "accept":
      if (state === "verifying") return "ready";
      break;
    case "begin_finalize":
      if (state === "ready") return "finalizing";
      break;
    case "commit_finalize":
      if (state === "finalizing") return "finalized";
      break;
    case "cancel":
      if (
        state === "created" ||
        state === "queued" ||
        state === "ready" ||
        state === "transferring" ||
        state === "verifying"
      ) {
        return "canceled";
      }
      break;
    case "reject":
      if (state === "verifying") return "rejected";
      break;
    case "expire":
      if (
        state === "created" ||
        state === "queued" ||
        state === "ready" ||
        state === "transferring" ||
        state === "verifying"
      ) {
        return "expired";
      }
      break;
    case "fail":
      return "failed";
    default:
      return assertNever(transition);
  }
  throw new UploadProtocolStateError("invalid_upload_transition");
}

function assertNever(value: never): never {
  void value;
  throw new UploadProtocolStateError("invalid_upload_transition");
}

export class UploadProtocolStateMachine {
  readonly #outcomes = new Map<
    string,
    Readonly<{
      expectedRevision: bigint;
      outcome: UploadProtocolTransitionOutcome;
      transition: UploadProtocolTransition;
    }>
  >();
  #revision: bigint;
  #state: UploadProtocolState;

  constructor(state: UploadProtocolState, revision: bigint) {
    this.#state = parseUploadProtocolState(state);
    if (revision < 0n || revision > MAX_U64) {
      throw new UploadProtocolStateError("revision_exhausted");
    }
    this.#revision = revision;
  }

  get state(): UploadProtocolState {
    return this.#state;
  }

  get revision(): bigint {
    return this.#revision;
  }

  apply(request: UploadProtocolTransitionRequest): UploadProtocolTransitionOutcome {
    const transition = parseUploadProtocolTransition(request.transition);
    const existing = this.#outcomes.get(request.idempotencyKey);
    if (existing !== undefined) {
      if (
        existing.expectedRevision !== request.expectedRevision ||
        existing.transition !== transition
      ) {
        throw new UploadProtocolStateError("upload_conflict");
      }
      return Object.freeze({ ...existing.outcome, disposition: "existing_outcome" });
    }
    if (request.expectedRevision !== this.#revision) {
      throw new UploadProtocolStateError("upload_conflict");
    }
    if (this.#outcomes.size === MAX_RETAINED_OUTCOMES) {
      throw new UploadProtocolStateError("upload_idempotency_history_full");
    }
    if (this.#revision === MAX_U64) {
      throw new UploadProtocolStateError("revision_exhausted");
    }
    const state = nextState(this.#state, transition);
    const outcome = Object.freeze({
      disposition: "applied" as const,
      revision: this.#revision + 1n,
      state,
    });
    this.#outcomes.set(
      request.idempotencyKey,
      Object.freeze({
        expectedRevision: request.expectedRevision,
        outcome,
        transition,
      }),
    );
    this.#revision = outcome.revision;
    this.#state = state;
    return outcome;
  }
}
