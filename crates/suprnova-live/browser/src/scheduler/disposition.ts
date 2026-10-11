import type { IntentDisposition, SchedulerTicket } from "./types.js";

export class DispositionLedger {
  readonly #maximum: number;
  readonly #entries = new Map<SchedulerTicket, IntentDisposition>();

  constructor(maximum: number) {
    this.#maximum = maximum;
  }

  record(ticket: SchedulerTicket, disposition: IntentDisposition): void {
    this.#entries.delete(ticket);
    this.#entries.set(ticket, disposition);
    while (this.#entries.size > this.#maximum) {
      const oldest = this.#entries.keys().next().value;
      if (oldest === undefined) break;
      this.#entries.delete(oldest);
    }
  }

  get(ticket: SchedulerTicket): IntentDisposition | undefined {
    return this.#entries.get(ticket);
  }

  size(): number {
    return this.#entries.size;
  }
}
