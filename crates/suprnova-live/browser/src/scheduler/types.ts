import type { ServerIntent } from "./intent.js";

export type SchedulerPolicy =
  | Readonly<{ kind: "fifo" }>
  | Readonly<{ kind: "replace_pending"; key: string }>
  | Readonly<{ kind: "drop_duplicate"; key: string }>
  | Readonly<{ kind: "latest_only"; key: string; abortInFlight: boolean }>
  | Readonly<{ kind: "parallel"; group: string; maximum: number }>;

export type IntentDisposition =
  | "accepted"
  | "rejected"
  | "duplicate"
  | "canceled"
  | "superseded"
  | "stale"
  | "out_of_order"
  | "incompatible"
  | "retired";

export interface SchedulerTicket {
  readonly sequence: number;
  readonly intent: ServerIntent;
  readonly policy: SchedulerPolicy;
}

export interface ScheduleResult {
  readonly disposition: IntentDisposition;
  readonly ticket?: SchedulerTicket;
}

export interface SchedulerOptions {
  readonly maxQueued: number;
  readonly maxParallel: number;
  readonly maxCompleted: number;
  readonly maxRecoveries: number;
}

export interface SchedulerCancelOptions {
  readonly abortTransport?: boolean;
}

export interface SchedulerSnapshot {
  readonly queued: number;
  readonly inFlight: number;
  readonly responseReady: number;
  readonly applying: number;
  readonly completedRetained: number;
  readonly recoveries: number;
  readonly retired: boolean;
}
