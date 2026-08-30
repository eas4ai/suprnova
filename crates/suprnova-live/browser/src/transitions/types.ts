export const MAX_TRANSITION_TARGETS = 64;
export const MAX_TRANSITION_NAME_BYTES = 64;
export const MAX_TRANSITION_DURATION_MS = 5_000;

export type TransitionKind = "enter" | "leave" | "move" | "state";

export interface TransitionSpec {
  readonly kind: TransitionKind;
  readonly name: string;
  readonly maximumMs: number;
  readonly essential: boolean;
}

export interface TransitionTarget {
  readonly element: Element;
  readonly spec: TransitionSpec;
  readonly applyFinalState: VoidFunction;
}

export interface TransitionHandle {
  readonly finished: Promise<void>;
  cancel(): void;
}

export interface TransitionCompletion {
  start(element: Element, spec: TransitionSpec): TransitionHandle | null;
}

export interface TransitionScheduler {
  timeout(callback: VoidFunction, milliseconds: number): number;
  clearTimeout(handle: number): void;
}

export type TransitionCancelReason = "canceled" | "superseded" | "navigation" | "removed";

export type TransitionStatus =
  "completed" | "reduced_motion" | "unsupported" | "timed_out" | "failed" | TransitionCancelReason;

export interface TransitionOutcome {
  readonly kind: TransitionKind;
  readonly name: string;
  readonly status: TransitionStatus;
}

export interface TransitionRun {
  readonly epoch: number;
  readonly finished: Promise<readonly TransitionOutcome[]>;
  cancel(reason?: TransitionCancelReason): void;
}
