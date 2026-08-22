import { DispositionLedger } from "./disposition.js";
import type { IntentFinishReason, ServerIntent } from "./intent.js";
import {
  FIFO_POLICY,
  normalizeSchedulerPolicy,
  schedulerPolicyEffects,
  schedulerPolicyKey,
} from "./policy.js";
import { createSchedulerRecord, phaseCount, type SchedulerRecord } from "./state.js";
import type {
  IntentDisposition,
  ScheduleResult,
  SchedulerCancelOptions,
  SchedulerOptions,
  SchedulerPolicy,
  SchedulerSnapshot,
  SchedulerTicket,
} from "./types.js";

const MAX_QUEUED = 1_024;
const MAX_PARALLEL = 32;
const MAX_COMPLETED = 4_096;
const MAX_RECOVERIES = 16;
const MAX_FEEDBACK_OBSERVERS = 8;

export interface SchedulerTransportFeedback {
  readonly retrying: boolean;
  readonly offline: boolean;
}

export interface SchedulerFeedbackRecord {
  readonly intentId: string;
  readonly fields: readonly string[];
  readonly actions: readonly string[];
  readonly phase: "pending" | "in_flight" | "response_ready" | "applying" | "completed";
  readonly disposition: IntentDisposition | null;
  readonly retrying: boolean;
  readonly offline: boolean;
}

function boundedInteger(value: number, minimum: number, maximum: number): boolean {
  return Number.isSafeInteger(value) && value >= minimum && value <= maximum;
}

function finishReason(disposition: IntentDisposition): IntentFinishReason {
  switch (disposition) {
    case "accepted":
      return "accepted";
    case "rejected":
    case "duplicate":
      return "rejected";
    case "canceled":
    case "superseded":
    case "retired":
      return "canceled";
    case "stale":
    case "out_of_order":
    case "incompatible":
      return "terminal";
  }
}

function safeAbort(abort: VoidFunction | null): void {
  if (abort === null) return;
  try {
    abort();
  } catch {
    // A transport abort port cannot change scheduler state or leak its failure.
  }
}

function semanticScopes(ticket: SchedulerTicket): {
  readonly fields: readonly string[];
  readonly actions: readonly string[];
} {
  const fields: string[] = [];
  const actions: string[] = [];
  for (const operation of ticket.intent.operations) {
    if (operation.kind === "sync_model") fields.push(operation.field);
    if (operation.kind === "invoke_action") actions.push(operation.name);
  }
  return Object.freeze({ actions: Object.freeze(actions), fields: Object.freeze(fields) });
}

function feedbackRecord(
  record: SchedulerRecord,
  transport: SchedulerTransportFeedback | undefined,
  disposition: IntentDisposition | null = null,
): SchedulerFeedbackRecord {
  const scopes = semanticScopes(record.ticket);
  return Object.freeze({
    actions: scopes.actions,
    disposition,
    fields: scopes.fields,
    intentId: String(record.ticket.sequence + 1),
    offline: transport?.offline ?? false,
    phase: disposition === null ? record.phase : "completed",
    retrying: transport?.retrying ?? false,
  });
}

export class IslandScheduler {
  readonly #maxQueued: number;
  readonly #maxParallel: number;
  readonly #maxRecoveries: number;
  readonly #completed: DispositionLedger;
  readonly #records: SchedulerRecord[] = [];
  readonly #byTicket = new Map<SchedulerTicket, SchedulerRecord>();
  readonly #seenIntents = new WeakSet<ServerIntent>();
  readonly #feedbackObservers = new Set<VoidFunction>();
  readonly #transportFeedback = new Map<SchedulerTicket, SchedulerTransportFeedback>();
  #terminalFeedback: SchedulerFeedbackRecord | null = null;
  #sequence = 0;
  #recoveries = 0;
  #retired = false;

  constructor(options: SchedulerOptions) {
    if (
      !boundedInteger(options.maxQueued, 1, MAX_QUEUED) ||
      !boundedInteger(options.maxParallel, 1, MAX_PARALLEL) ||
      !boundedInteger(options.maxCompleted, 1, MAX_COMPLETED) ||
      !boundedInteger(options.maxRecoveries, 1, MAX_RECOVERIES)
    ) {
      throw new Error("scheduler_options_invalid");
    }
    this.#maxQueued = options.maxQueued;
    this.#maxParallel = options.maxParallel;
    this.#maxRecoveries = options.maxRecoveries;
    this.#completed = new DispositionLedger(options.maxCompleted);
  }

  subscribeFeedback(observer: VoidFunction): VoidFunction {
    if (this.#feedbackObservers.size >= MAX_FEEDBACK_OBSERVERS) {
      throw new Error("scheduler_feedback_observer_limit");
    }
    this.#feedbackObservers.add(observer);
    let subscribed = true;
    return () => {
      if (!subscribed) return;
      subscribed = false;
      this.#feedbackObservers.delete(observer);
    };
  }

  feedback(): readonly SchedulerFeedbackRecord[] {
    const records = this.#records.map((record) =>
      feedbackRecord(record, this.#transportFeedback.get(record.ticket)),
    );
    if (this.#terminalFeedback !== null) records.push(this.#terminalFeedback);
    return Object.freeze(records);
  }

  setTransportFeedback(
    ticket: SchedulerTicket,
    feedback: SchedulerTransportFeedback,
  ): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (record.phase !== "in_flight") return "incompatible";
    const normalized = Object.freeze({
      offline: feedback.offline,
      retrying: feedback.retrying,
    });
    this.#transportFeedback.set(ticket, normalized);
    this.#emitFeedback();
    return "accepted";
  }

  schedule(intent: ServerIntent, policy: SchedulerPolicy = FIFO_POLICY): ScheduleResult {
    if (this.#retired) {
      this.#finishUnscheduled(intent, "retired");
      return Object.freeze({ disposition: "retired" });
    }
    if (this.#seenIntents.has(intent)) {
      return Object.freeze({ disposition: "duplicate" });
    }
    this.#seenIntents.add(intent);
    let normalized: SchedulerPolicy;
    try {
      normalized = normalizeSchedulerPolicy(policy);
      if (normalized.kind === "parallel" && normalized.maximum > this.#maxParallel) {
        throw new Error("scheduler_parallel_limit");
      }
    } catch {
      this.#finishUnscheduled(intent, "rejected");
      return Object.freeze({ disposition: "rejected" });
    }

    const effects = schedulerPolicyEffects(normalized);
    const key = schedulerPolicyKey(normalized);
    const matching =
      key === null
        ? []
        : this.#records.filter((record) => schedulerPolicyKey(record.ticket.policy) === key);
    if (effects.duplicatePending && matching.length > 0) {
      this.#finishUnscheduled(intent, "duplicate");
      return Object.freeze({ disposition: "duplicate" });
    }
    if (effects.replacePending) {
      for (const record of [...matching]) {
        if (record.phase === "pending" || record.phase === "response_ready") {
          this.#finalize(record, "superseded");
        } else if (record.phase === "in_flight" && effects.supersedeInFlight) {
          if (effects.abortInFlight) {
            const abort = record.abort;
            record.abort = null;
            this.#finalize(record, "superseded");
            safeAbort(abort);
          } else {
            record.applicationEligible = false;
            record.suppressedDisposition = "superseded";
          }
        }
      }
    }
    if (phaseCount(this.#records, "pending") >= this.#maxQueued) {
      this.#finishUnscheduled(intent, "rejected");
      return Object.freeze({ disposition: "rejected" });
    }
    if (this.#sequence >= Number.MAX_SAFE_INTEGER) {
      this.#finishUnscheduled(intent, "rejected");
      return Object.freeze({ disposition: "rejected" });
    }
    const ticket: SchedulerTicket = Object.freeze({
      intent,
      policy: normalized,
      sequence: this.#sequence,
    });
    this.#sequence += 1;
    const record = createSchedulerRecord(ticket);
    this.#terminalFeedback = null;
    this.#records.push(record);
    this.#byTicket.set(ticket, record);
    this.#emitFeedback();
    return Object.freeze({ disposition: "accepted", ticket });
  }

  ready(): readonly SchedulerTicket[] {
    if (this.#retired || this.#records.length === 0) return Object.freeze([]);
    const first = this.#records[0];
    if (first === undefined) return Object.freeze([]);
    if (first.ticket.policy.kind !== "parallel") {
      return Object.freeze(first.phase === "pending" ? [first.ticket] : []);
    }

    const group = first.ticket.policy.group;
    let maximum = Math.min(this.#maxParallel, first.ticket.policy.maximum);
    let active = 0;
    const pending: SchedulerTicket[] = [];
    for (const record of this.#records) {
      if (record.ticket.policy.kind !== "parallel" || record.ticket.policy.group !== group) break;
      maximum = Math.min(maximum, record.ticket.policy.maximum);
      if (record.phase !== "pending") active += 1;
      if (record.phase === "pending") pending.push(record.ticket);
    }
    return Object.freeze(pending.slice(0, Math.max(0, maximum - active)));
  }

  start(ticket: SchedulerTicket, abort: VoidFunction = () => undefined): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (record.phase !== "pending") return "incompatible";
    if (!this.ready().includes(ticket) || typeof abort !== "function") return "incompatible";
    record.phase = "in_flight";
    record.abort = abort;
    this.#transportFeedback.delete(ticket);
    this.#emitFeedback();
    return "accepted";
  }

  settleTransport(ticket: SchedulerTicket): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (record.phase !== "in_flight") {
      return record.phase === "response_ready" || record.phase === "applying"
        ? "duplicate"
        : "incompatible";
    }
    record.abort = null;
    if (!record.applicationEligible) {
      const disposition = record.suppressedDisposition ?? "stale";
      this.#finalize(record, disposition);
      return disposition;
    }
    record.phase = "response_ready";
    this.#transportFeedback.delete(ticket);
    this.#emitFeedback();
    return "accepted";
  }

  beginApplication(ticket: SchedulerTicket): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (record.phase !== "response_ready") {
      return record.phase === "applying" ? "duplicate" : "incompatible";
    }
    if (!record.applicationEligible) {
      const disposition = record.suppressedDisposition ?? "stale";
      this.#finalize(record, disposition);
      return disposition;
    }
    const earliest = this.#records.find((candidate) => candidate.applicationEligible);
    if (earliest !== record) return "out_of_order";
    record.phase = "applying";
    this.#emitFeedback();
    return "accepted";
  }

  finish(ticket: SchedulerTicket, disposition: IntentDisposition): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (disposition === "accepted" && record.phase !== "applying") return "incompatible";
    this.#finalize(record, disposition);
    return disposition;
  }

  cancel(ticket: SchedulerTicket, options: SchedulerCancelOptions = {}): IntentDisposition {
    const record = this.#byTicket.get(ticket);
    if (record === undefined) return this.#terminal(ticket);
    if (record.phase === "applying") return "incompatible";
    if (record.phase === "in_flight" && options.abortTransport !== true) {
      record.applicationEligible = false;
      record.suppressedDisposition = "canceled";
      this.#emitFeedback();
      return "canceled";
    }
    const abort = record.phase === "in_flight" ? record.abort : null;
    record.abort = null;
    this.#finalize(record, "canceled");
    safeAbort(abort);
    return "canceled";
  }

  claimRecovery(): boolean {
    if (this.#retired || this.#recoveries >= this.#maxRecoveries) return false;
    this.#recoveries += 1;
    return true;
  }

  resetRecovery(): void {
    if (!this.#retired) this.#recoveries = 0;
  }

  retire(): void {
    if (this.#retired) return;
    this.#retired = true;
    for (const record of [...this.#records]) {
      const abort = record.phase === "in_flight" ? record.abort : null;
      record.abort = null;
      this.#finalize(record, "retired");
      safeAbort(abort);
    }
  }

  snapshot(): SchedulerSnapshot {
    return Object.freeze({
      applying: phaseCount(this.#records, "applying"),
      completedRetained: this.#completed.size(),
      inFlight: phaseCount(this.#records, "in_flight"),
      queued: phaseCount(this.#records, "pending"),
      recoveries: this.#recoveries,
      responseReady: phaseCount(this.#records, "response_ready"),
      retired: this.#retired,
    });
  }

  #finalize(record: SchedulerRecord, disposition: IntentDisposition): void {
    const transport = this.#transportFeedback.get(record.ticket);
    const index = this.#records.indexOf(record);
    if (index >= 0) this.#records.splice(index, 1);
    this.#byTicket.delete(record.ticket);
    this.#transportFeedback.delete(record.ticket);
    this.#completed.record(record.ticket, disposition);
    this.#terminalFeedback = feedbackRecord(record, transport, disposition);
    record.ticket.intent.finish(finishReason(disposition));
    this.#emitFeedback();
  }

  #finishUnscheduled(intent: ServerIntent, disposition: IntentDisposition): void {
    intent.finish(finishReason(disposition));
  }

  #terminal(ticket: SchedulerTicket): IntentDisposition {
    return this.#completed.get(ticket) ?? (this.#retired ? "retired" : "stale");
  }

  #emitFeedback(): void {
    for (const observer of this.#feedbackObservers) {
      try {
        observer();
      } catch {
        // Presentation observers cannot change scheduler authority.
      }
    }
  }
}
