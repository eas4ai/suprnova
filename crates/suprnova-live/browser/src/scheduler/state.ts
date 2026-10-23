import type { IntentDisposition, SchedulerTicket } from "./types.js";

export type SchedulerPhase = "pending" | "in_flight" | "response_ready" | "applying";

export interface SchedulerRecord {
  readonly ticket: SchedulerTicket;
  phase: SchedulerPhase;
  abort: VoidFunction | null;
  applicationEligible: boolean;
  suppressedDisposition: IntentDisposition | null;
}

export function createSchedulerRecord(ticket: SchedulerTicket): SchedulerRecord {
  return {
    abort: null,
    applicationEligible: true,
    phase: "pending",
    suppressedDisposition: null,
    ticket,
  };
}

export function phaseCount(records: readonly SchedulerRecord[], phase: SchedulerPhase): number {
  let count = 0;
  for (const record of records) if (record.phase === phase) count += 1;
  return count;
}
