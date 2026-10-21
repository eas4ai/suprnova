import { describe, expect, it } from "vitest";

import type { IntentSource } from "../src/scheduler/intent.js";
import { ServerIntent } from "../src/scheduler/intent.js";
import { IslandScheduler } from "../src/scheduler/scheduler.js";
import type { ScheduleResult, SchedulerPolicy, SchedulerTicket } from "../src/scheduler/types.js";

const FIFO: SchedulerPolicy = Object.freeze({ kind: "fifo" });

function intent(name: string): { readonly value: ServerIntent; finishes(): number } {
  let finishes = 0;
  const value = new ServerIntent(
    Object.freeze({ eventType: name }) as unknown as IntentSource,
    [Object.freeze({ kind: "fresh_render" })],
    null,
  );
  value.onFinish(() => {
    finishes += 1;
  });
  return { value, finishes: () => finishes };
}

function ticket(result: ScheduleResult): SchedulerTicket {
  expect(result.disposition).toBe("accepted");
  if (result.ticket === undefined) throw new Error("missing scheduler ticket");
  return result.ticket;
}

function scheduler(overrides: Partial<ConstructorParameters<typeof IslandScheduler>[0]> = {}) {
  return new IslandScheduler({
    maxCompleted: 8,
    maxParallel: 2,
    maxQueued: 3,
    maxRecoveries: 2,
    ...overrides,
  });
}

describe("bounded island scheduler", () => {
  it("runs FIFO transport and application in intent order", () => {
    const work = scheduler();
    const first = intent("first");
    const second = intent("second");
    const firstTicket = ticket(work.schedule(first.value, FIFO));
    const secondTicket = ticket(work.schedule(second.value, FIFO));

    expect(work.ready()).toEqual([firstTicket]);
    expect(work.start(firstTicket)).toBe("accepted");
    expect(work.ready()).toEqual([]);
    expect(work.settleTransport(firstTicket)).toBe("accepted");
    expect(work.beginApplication(secondTicket)).toBe("incompatible");
    expect(work.beginApplication(firstTicket)).toBe("accepted");
    expect(work.finish(firstTicket, "accepted")).toBe("accepted");
    expect(first.finishes()).toBe(1);
    expect(work.ready()).toEqual([secondTicket]);
  });

  it("rejects queue overflow without evicting in-flight authority", () => {
    const work = scheduler({ maxQueued: 2 });
    const first = intent("first");
    const second = intent("second");
    const third = intent("third");
    const firstTicket = ticket(work.schedule(first.value, FIFO));
    ticket(work.schedule(second.value, FIFO));
    expect(work.schedule(third.value, FIFO).disposition).toBe("rejected");
    expect(third.finishes()).toBe(1);

    expect(work.start(firstTicket)).toBe("accepted");
    const replacement = intent("replacement");
    expect(work.schedule(replacement.value, FIFO).disposition).toBe("accepted");
    expect(work.snapshot()).toMatchObject({ inFlight: 1, queued: 2 });
  });

  it("drops duplicates and supersedes only matching pending replacement work", () => {
    const work = scheduler();
    const duplicatePolicy: SchedulerPolicy = Object.freeze({
      kind: "drop_duplicate",
      key: "search",
    });
    const first = intent("first");
    const duplicate = intent("duplicate");
    ticket(work.schedule(first.value, duplicatePolicy));
    expect(work.schedule(duplicate.value, duplicatePolicy).disposition).toBe("duplicate");
    expect(duplicate.finishes()).toBe(1);

    const replacementWork = scheduler();
    const replacePolicy: SchedulerPolicy = Object.freeze({
      kind: "replace_pending",
      key: "filter",
    });
    const oldPending = intent("old");
    const oldTicket = ticket(replacementWork.schedule(oldPending.value, replacePolicy));
    const newest = intent("new");
    const newestTicket = ticket(replacementWork.schedule(newest.value, replacePolicy));
    expect(replacementWork.start(oldTicket)).toBe("superseded");
    expect(oldPending.finishes()).toBe(1);
    expect(replacementWork.ready()).toContain(newestTicket);
  });

  it("makes latest-only in-flight work ineligible and aborts only when declared", () => {
    const passive = scheduler();
    const policy: SchedulerPolicy = Object.freeze({
      abortInFlight: false,
      key: "query",
      kind: "latest_only",
    });
    const old = intent("old");
    const oldTicket = ticket(passive.schedule(old.value, policy));
    expect(passive.start(oldTicket)).toBe("accepted");
    const newest = intent("new");
    const newestTicket = ticket(passive.schedule(newest.value, policy));
    expect(passive.ready()).toEqual([]);
    expect(passive.settleTransport(oldTicket)).toBe("superseded");
    expect(passive.ready()).toEqual([newestTicket]);

    const aborting = scheduler();
    const abortPolicy = Object.freeze({ ...policy, abortInFlight: true });
    const aborted = intent("aborted");
    const abortedTicket = ticket(aborting.schedule(aborted.value, abortPolicy));
    let aborts = 0;
    expect(
      aborting.start(abortedTicket, () => {
        aborts += 1;
      }),
    ).toBe("accepted");
    const next = intent("next");
    const nextTicket = ticket(aborting.schedule(next.value, abortPolicy));
    expect(aborts).toBe(1);
    expect(aborting.settleTransport(abortedTicket)).toBe("superseded");
    expect(aborting.ready()).toEqual([nextTicket]);
  });

  it("allows declared parallel transport but serializes response application", () => {
    const work = scheduler({ maxParallel: 3, maxQueued: 4 });
    const parallel: SchedulerPolicy = Object.freeze({
      group: "metrics",
      kind: "parallel",
      maximum: 2,
    });
    const first = intent("first");
    const second = intent("second");
    const third = intent("third");
    const barrier = intent("barrier");
    const firstTicket = ticket(work.schedule(first.value, parallel));
    const secondTicket = ticket(work.schedule(second.value, parallel));
    const thirdTicket = ticket(work.schedule(third.value, parallel));
    const barrierTicket = ticket(work.schedule(barrier.value, FIFO));

    expect(work.ready()).toEqual([firstTicket, secondTicket]);
    expect(work.start(firstTicket)).toBe("accepted");
    expect(work.start(secondTicket)).toBe("accepted");
    expect(work.ready()).toEqual([]);
    expect(work.settleTransport(secondTicket)).toBe("accepted");
    expect(work.ready()).toEqual([]);
    expect(work.beginApplication(secondTicket)).toBe("out_of_order");
    expect(work.settleTransport(firstTicket)).toBe("accepted");
    expect(work.ready()).toEqual([]);
    expect(work.beginApplication(firstTicket)).toBe("accepted");
    expect(work.finish(firstTicket, "accepted")).toBe("accepted");
    expect(work.ready()).toEqual([thirdTicket]);
    expect(work.start(thirdTicket)).toBe("accepted");
    expect(work.settleTransport(thirdTicket)).toBe("accepted");
    expect(work.beginApplication(secondTicket)).toBe("accepted");
    expect(work.finish(secondTicket, "accepted")).toBe("accepted");
    expect(work.beginApplication(thirdTicket)).toBe("accepted");
    expect(work.finish(thirdTicket, "accepted")).toBe("accepted");
    expect(work.ready()).toEqual([barrierTicket]);
  });

  it("cancels pending and in-flight work without making late callbacks eligible", () => {
    const work = scheduler();
    const pending = intent("pending");
    const pendingTicket = ticket(work.schedule(pending.value, FIFO));
    expect(work.cancel(pendingTicket)).toBe("canceled");
    expect(work.start(pendingTicket)).toBe("canceled");

    const active = intent("active");
    const activeTicket = ticket(work.schedule(active.value, FIFO));
    let aborts = 0;
    work.start(activeTicket, () => {
      aborts += 1;
    });
    expect(work.cancel(activeTicket, { abortTransport: true })).toBe("canceled");
    expect(aborts).toBe(1);
    expect(work.settleTransport(activeTicket)).toBe("canceled");
    expect(work.beginApplication(activeTicket)).toBe("canceled");
    expect(active.finishes()).toBe(1);
  });

  it("retires all work exactly once and bounds recovery and disposition retention", () => {
    const work = scheduler({ maxCompleted: 2, maxRecoveries: 2 });
    const first = intent("first");
    const second = intent("second");
    const firstTicket = ticket(work.schedule(first.value, FIFO));
    const secondTicket = ticket(work.schedule(second.value, FIFO));
    let aborts = 0;
    work.start(firstTicket, () => {
      aborts += 1;
    });
    expect(work.claimRecovery()).toBe(true);
    expect(work.claimRecovery()).toBe(true);
    expect(work.claimRecovery()).toBe(false);
    work.retire();
    work.retire();

    expect(aborts).toBe(1);
    expect(first.finishes()).toBe(1);
    expect(second.finishes()).toBe(1);
    expect(work.settleTransport(firstTicket)).toBe("retired");
    expect(work.start(secondTicket)).toBe("retired");
    expect(work.schedule(intent("late").value, FIFO).disposition).toBe("retired");
    expect(work.snapshot()).toMatchObject({ applying: 0, inFlight: 0, queued: 0, retired: true });
  });

  it("keeps independent islands responsive", () => {
    const firstIsland = scheduler();
    const secondIsland = scheduler();
    const firstTicket = ticket(firstIsland.schedule(intent("first").value, FIFO));
    const secondTicket = ticket(secondIsland.schedule(intent("second").value, FIFO));
    firstIsland.start(firstTicket);
    expect(firstIsland.ready()).toEqual([]);
    expect(secondIsland.ready()).toEqual([secondTicket]);
  });

  it("bounds and isolates intent completion callbacks", () => {
    const bounded = intent("bounded").value;
    for (let index = 0; index < 63; index += 1) bounded.onFinish(() => undefined);
    expect(() => {
      bounded.onFinish(() => undefined);
    }).toThrow("intent_finish_callback_limit");

    const isolated = intent("isolated").value;
    let tail = 0;
    isolated.onFinish(() => {
      throw new Error("callback detail must not escape");
    });
    isolated.onFinish(() => {
      tail += 1;
    });
    expect(() => {
      isolated.finish("canceled");
    }).not.toThrow();
    expect(tail).toBe(1);
  });
});
