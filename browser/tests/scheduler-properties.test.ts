import fc from "fast-check";
import { describe, expect, it } from "vitest";

import type { IntentSource } from "../src/scheduler/intent.js";
import { ServerIntent } from "../src/scheduler/intent.js";
import { IslandScheduler } from "../src/scheduler/scheduler.js";
import type { SchedulerPolicy, SchedulerTicket } from "../src/scheduler/types.js";

interface Model {
  retired: boolean;
}

interface Real {
  readonly scheduler: IslandScheduler;
  readonly tickets: SchedulerTicket[];
  next: number;
}

function newIntent(index: number): ServerIntent {
  return new ServerIntent(
    Object.freeze({ eventType: `property-${String(index)}` }) as unknown as IntentSource,
    [Object.freeze({ kind: "fresh_render" })],
    null,
  );
}

function invariant(model: Model, real: Real): void {
  const snapshot = real.scheduler.snapshot();
  expect(snapshot.queued).toBeLessThanOrEqual(4);
  expect(snapshot.inFlight).toBeLessThanOrEqual(3);
  expect(snapshot.applying).toBeLessThanOrEqual(1);
  expect(snapshot.retired).toBe(model.retired);
  if (model.retired) expect(real.scheduler.ready()).toEqual([]);
}

class ScheduleCommand implements fc.Command<Model, Real> {
  constructor(readonly policy: SchedulerPolicy) {}
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    const result = real.scheduler.schedule(newIntent(real.next), this.policy);
    real.next += 1;
    if (result.ticket !== undefined) real.tickets.push(result.ticket);
    if (model.retired) expect(result.disposition).toBe("retired");
    invariant(model, real);
  }
  toString(): string {
    return `schedule:${this.policy.kind}`;
  }
}

class StartCommand implements fc.Command<Model, Real> {
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    const candidate = real.scheduler.ready()[0];
    if (candidate !== undefined) real.scheduler.start(candidate, () => undefined);
    invariant(model, real);
  }
  toString(): string {
    return "start";
  }
}

class SettleCommand implements fc.Command<Model, Real> {
  constructor(readonly index: number) {}
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    const candidate = real.tickets[this.index % Math.max(1, real.tickets.length)];
    if (candidate !== undefined) real.scheduler.settleTransport(candidate);
    invariant(model, real);
  }
  toString(): string {
    return "settle";
  }
}

class ApplyCommand implements fc.Command<Model, Real> {
  constructor(readonly index: number) {}
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    const candidate = real.tickets[this.index % Math.max(1, real.tickets.length)];
    if (candidate !== undefined && real.scheduler.beginApplication(candidate) === "accepted") {
      real.scheduler.finish(candidate, "accepted");
    }
    invariant(model, real);
  }
  toString(): string {
    return "apply";
  }
}

class CancelCommand implements fc.Command<Model, Real> {
  constructor(
    readonly index: number,
    readonly abortTransport: boolean,
  ) {}
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    const candidate = real.tickets[this.index % Math.max(1, real.tickets.length)];
    if (candidate !== undefined)
      real.scheduler.cancel(candidate, { abortTransport: this.abortTransport });
    invariant(model, real);
  }
  toString(): string {
    return "cancel";
  }
}

class RetireCommand implements fc.Command<Model, Real> {
  check(): boolean {
    return true;
  }
  run(model: Model, real: Real): void {
    real.scheduler.retire();
    model.retired = true;
    invariant(model, real);
  }
  toString(): string {
    return "retire";
  }
}

const policy = fc.oneof(
  fc.constant(Object.freeze({ kind: "fifo" }) as SchedulerPolicy),
  fc.constant(Object.freeze({ key: "query", kind: "drop_duplicate" }) as SchedulerPolicy),
  fc.constant(Object.freeze({ key: "query", kind: "replace_pending" }) as SchedulerPolicy),
  fc
    .boolean()
    .map(
      (abortInFlight) =>
        Object.freeze({ abortInFlight, key: "query", kind: "latest_only" }) as SchedulerPolicy,
    ),
  fc
    .integer({ max: 3, min: 1 })
    .map(
      (maximum) => Object.freeze({ group: "safe", kind: "parallel", maximum }) as SchedulerPolicy,
    ),
);

const command = fc.oneof(
  policy.map((value) => new ScheduleCommand(value)),
  fc.constant(new StartCommand()),
  fc.nat(32).map((index) => new SettleCommand(index)),
  fc.nat(32).map((index) => new ApplyCommand(index)),
  fc
    .tuple(fc.nat(32), fc.boolean())
    .map(([index, abortTransport]) => new CancelCommand(index, abortTransport)),
  fc.constant(new RetireCommand()),
);

describe("scheduler command model", () => {
  it("preserves bounds and retirement under adversarial callback order", () => {
    fc.assert(
      fc.property(fc.commands([command], { maxCommands: 100 }), (commands) => {
        fc.modelRun(
          () => ({
            model: { retired: false },
            real: {
              next: 0,
              scheduler: new IslandScheduler({
                maxCompleted: 16,
                maxParallel: 3,
                maxQueued: 4,
                maxRecoveries: 2,
              }),
              tickets: [],
            },
          }),
          commands,
        );
      }),
      { numRuns: 200 },
    );
  });
});
