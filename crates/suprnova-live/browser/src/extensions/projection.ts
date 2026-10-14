import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import type { RuntimeScheduler } from "../runtime/ports.js";

const MAX_ACTIVE_PROJECTIONS = 128;
const MAX_PATCHES = 32;
const MAX_DECLARATIONS = 64;
const DECLARATION = /^(?:attr|class|expanded|inert|selected|show):[A-Za-z][A-Za-z0-9_-]{0,127}$/u;

export interface ProjectionIntent {
  onFinish(callback: VoidFunction): void;
}

export interface ProjectionPatch {
  readonly declaration: string;
  connected(): boolean;
  applyPending(): void;
  rollback(): void;
}

export type ProjectionSettlement =
  "accepted_html" | "accepted_no_render" | "rejected" | "interrupted" | "canceled";

export type ProjectionState = "pending" | "settled" | "recovery_required";

export interface ProjectionHandle {
  state(): ProjectionState;
  settle(outcome: ProjectionSettlement): ProjectionState;
}

interface ProjectionRecord {
  state: ProjectionState;
  readonly intent: ProjectionIntent;
  readonly patches: readonly ProjectionPatch[];
  timeout: number;
}

export interface OptimisticProjectionOptions {
  readonly scheduler: RuntimeScheduler;
  readonly diagnostics: RuntimeDiagnosticSink;
  readonly timeoutMs: number;
}

export class OptimisticProjectionManager {
  readonly #scheduler: RuntimeScheduler;
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #timeoutMs: number;
  readonly #active = new Map<ProjectionIntent, ProjectionRecord>();
  #disposed = false;

  constructor(options: OptimisticProjectionOptions) {
    if (
      !Number.isSafeInteger(options.timeoutMs) ||
      options.timeoutMs < 1 ||
      options.timeoutMs > 30_000
    ) {
      throw new Error("projection_timeout_invalid");
    }
    this.#scheduler = options.scheduler;
    this.#diagnostics = options.diagnostics;
    this.#timeoutMs = options.timeoutMs;
  }

  begin(
    intent: ProjectionIntent,
    declarations: ReadonlySet<string>,
    patches: readonly ProjectionPatch[],
  ): ProjectionHandle {
    if (
      this.#disposed ||
      !Object.isFrozen(intent) ||
      typeof intent.onFinish !== "function" ||
      this.#active.has(intent) ||
      this.#active.size >= MAX_ACTIVE_PROJECTIONS
    ) {
      throw new Error("projection_intent_invalid");
    }
    if (
      declarations.size === 0 ||
      declarations.size > MAX_DECLARATIONS ||
      [...declarations].some((declaration) => !DECLARATION.test(declaration)) ||
      patches.length === 0 ||
      patches.length > MAX_PATCHES
    ) {
      throw new Error("projection_limit");
    }
    if (patches.some((patch) => !declarations.has(patch.declaration))) {
      throw new Error("projection_target_undeclared");
    }
    const applied: ProjectionPatch[] = [];
    try {
      for (const patch of patches) {
        if (!patch.connected()) throw new Error("projection_target_removed");
        patch.applyPending();
        applied.push(patch);
      }
    } catch {
      for (const patch of applied.reverse()) {
        if (patch.connected()) patch.rollback();
      }
      this.#failure("recovery_required");
      throw new Error("projection_incompatible");
    }
    const record: ProjectionRecord = {
      intent,
      patches: Object.freeze([...patches]),
      state: "pending",
      timeout: 0,
    };
    this.#active.set(intent, record);
    const handle: ProjectionHandle = Object.freeze({
      state: () => record.state,
      settle: (outcome: ProjectionSettlement) => this.#settle(record, outcome),
    });
    record.timeout = this.#scheduler.timeout(() => {
      this.#settle(record, "interrupted");
    }, this.#timeoutMs);
    intent.onFinish(() => {
      this.#settle(record, "canceled");
    });
    return handle;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const record of [...this.#active.values()]) this.#settle(record, "canceled");
  }

  #settle(record: ProjectionRecord, outcome: ProjectionSettlement): ProjectionState {
    if (record.state !== "pending") return record.state;
    this.#scheduler.clearTimeout(record.timeout);
    this.#active.delete(record.intent);
    let recovery = false;
    if (outcome === "accepted_no_render") {
      recovery = record.patches.some((patch) => !patch.connected());
    } else if (outcome !== "accepted_html") {
      for (const patch of [...record.patches].reverse()) {
        if (!patch.connected()) {
          recovery = true;
          continue;
        }
        try {
          patch.rollback();
        } catch {
          recovery = true;
        }
      }
    }
    record.state = recovery ? "recovery_required" : "settled";
    if (recovery) this.#failure("recovery_required");
    return record.state;
  }

  #failure(detailCode: "operation_rejected" | "recovery_required"): void {
    this.#diagnostics.record({
      code: "effect_failed",
      severity: "error",
      phase: "effect",
      detailCode,
    });
  }
}
