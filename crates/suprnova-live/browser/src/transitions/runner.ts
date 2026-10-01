import {
  MAX_TRANSITION_DURATION_MS,
  MAX_TRANSITION_NAME_BYTES,
  MAX_TRANSITION_TARGETS,
  type TransitionCancelReason,
  type TransitionCompletion,
  type TransitionHandle,
  type TransitionOutcome,
  type TransitionRun,
  type TransitionScheduler,
  type TransitionSpec,
  type TransitionStatus,
  type TransitionTarget,
} from "./types.js";

const SAFE_NAME = /^[a-z][a-z0-9_-]*$/u;

export interface TransitionRunnerOptions {
  readonly completion: TransitionCompletion;
  readonly scheduler: TransitionScheduler;
  readonly prefersReducedMotion: () => boolean;
}

interface TargetRun {
  readonly finished: Promise<TransitionOutcome>;
  cancel(reason: TransitionCancelReason): void;
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function validSpec(spec: TransitionSpec): boolean {
  return (
    SAFE_NAME.test(spec.name) &&
    utf8Length(spec.name) <= MAX_TRANSITION_NAME_BYTES &&
    Number.isSafeInteger(spec.maximumMs) &&
    spec.maximumMs >= 0 &&
    spec.maximumMs <= MAX_TRANSITION_DURATION_MS
  );
}

export class TransitionRunner {
  readonly #completion: TransitionCompletion;
  readonly #scheduler: TransitionScheduler;
  readonly #prefersReducedMotion: () => boolean;
  #epoch = 0;

  constructor(options: TransitionRunnerOptions) {
    this.#completion = options.completion;
    this.#scheduler = options.scheduler;
    this.#prefersReducedMotion = options.prefersReducedMotion;
  }

  start(targets: readonly TransitionTarget[]): TransitionRun {
    if (targets.length > MAX_TRANSITION_TARGETS) throw new Error("transition_target_limit");
    if (this.#epoch >= Number.MAX_SAFE_INTEGER) throw new Error("transition_epoch_exhausted");
    for (const target of targets) {
      if (!validSpec(target.spec) || typeof target.applyFinalState !== "function") {
        throw new Error("transition_spec_invalid");
      }
    }
    const epoch = (this.#epoch += 1);
    const runs = targets.map((target) => this.#startTarget(target));
    let canceled = false;
    return Object.freeze({
      cancel: (reason: TransitionCancelReason = "canceled") => {
        if (canceled) return;
        canceled = true;
        for (const run of runs) run.cancel(reason);
      },
      epoch,
      finished: Promise.all(runs.map((run) => run.finished)).then((outcomes) =>
        Object.freeze(outcomes),
      ),
    });
  }

  #startTarget(target: TransitionTarget): TargetRun {
    let settle!: (outcome: TransitionOutcome) => void;
    const finished = new Promise<TransitionOutcome>((resolve) => {
      settle = resolve;
    });
    let handle: TransitionHandle | null = null;
    let timer: number | null = null;
    let settled = false;

    const finish = (status: TransitionStatus, cancelAnimation: boolean): void => {
      if (settled) return;
      settled = true;
      if (timer !== null) {
        try {
          this.#scheduler.clearTimeout(timer);
        } catch {
          // Timer cleanup cannot withhold semantic final state.
        }
        timer = null;
      }
      if (cancelAnimation && handle !== null) {
        try {
          handle.cancel();
        } catch {
          // Animation cleanup is best-effort after the run loses eligibility.
        }
      }
      try {
        target.applyFinalState();
      } catch {
        status = "failed";
      }
      settle(Object.freeze({ kind: target.spec.kind, name: target.spec.name, status }));
    };

    const reducedMotion = (() => {
      try {
        return this.#prefersReducedMotion();
      } catch {
        return true;
      }
    })();
    if (reducedMotion && !target.spec.essential) {
      finish("reduced_motion", false);
      return { cancel: () => undefined, finished };
    }

    try {
      handle = this.#completion.start(target.element, target.spec);
    } catch {
      finish("failed", false);
      return { cancel: () => undefined, finished };
    }
    if (handle === null) {
      finish("unsupported", false);
      return { cancel: () => undefined, finished };
    }

    try {
      timer = this.#scheduler.timeout(() => {
        finish("timed_out", true);
      }, target.spec.maximumMs);
    } catch {
      finish("failed", true);
      return { cancel: () => undefined, finished };
    }
    void handle.finished.then(
      () => {
        finish("completed", false);
      },
      () => {
        finish("failed", true);
      },
    );
    return {
      cancel: (reason) => {
        finish(reason, true);
      },
      finished,
    };
  }
}
