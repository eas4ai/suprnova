import { afterEach, describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";

type Timer = { callback: () => void; milliseconds: number };

type Visit = {
  except: string[];
  headers?: Record<string, string>;
  id: string;
  only: string[];
  reset: string[];
};

type RouterEvent = "before" | "error" | "finish" | "success";

type RouterListener = (event: {
  defaultPrevented: boolean;
  detail: {
    page?: unknown;
    visit?: Visit;
    visitId?: string;
  };
}) => void;

async function loadCore() {
  const url = new URL(
    "../src/templates/files/frontend/live/core.ts.tpl",
    import.meta.url,
  );
  const source = await readFile(url, "utf8");
  const javascript = stripTypeScriptTypes(source, { mode: "strip" });
  return import(
    `data:text/javascript;base64,${Buffer.from(javascript).toString("base64")}`
  );
}

function fixture() {
  const timers = new Map<number, Timer>();
  let nextTimer = 1;
  let nextVisit = 1;
  const listeners = new Map<RouterEvent, Set<RouterListener>>();
  let handlers: {
    onPoll: (
      topic: string,
      completion?: (
        outcome: "succeeded" | "failed" | "canceled" | "retired",
      ) => void,
    ) => void;
    onSignal: (signal: {
      topic: string;
      cursor: number;
      props: string[];
    }) => void;
    onState: (topic: string, state: string) => void;
  } | null = null;
  let closed = 0;
  let subscribedBindings: object = {};
  const reloads: Array<{
    headers?: Record<string, string>;
    only?: string[];
    onFinish?: () => void;
    onSuccess?: (page: unknown) => void;
  }> = [];
  let page = {
    liveBindings: {
      notes: {
        cursor: 10,
        island: {
          component: "kit.note-stats",
          document_key: "dashboard-notes",
          slot: "stats",
        },
        only: ["stats", "recent_notes"],
        stream: "changes",
      },
      profile: {
        cursor: 3,
        island: {
          component: "kit.profile",
          document_key: "dashboard-profile",
          slot: "auth",
        },
        only: ["auth"],
        stream: "changes",
      },
    },
    props: {},
  };

  return {
    beginVisit(overrides: Partial<Visit> = {}) {
      const visit: Visit = {
        except: [],
        headers: { Existing: "kept" },
        id: `visit-${nextVisit++}`,
        only: [],
        reset: [],
        ...overrides,
      };
      for (const listener of listeners.get("before") ?? []) {
        listener({ defaultPrevented: false, detail: { visit } });
      }
      return visit;
    },
    clearTimer(handle: number) {
      timers.delete(handle);
    },
    get closed() {
      return closed;
    },
    get subscribedBindings() {
      return subscribedBindings;
    },
    get handlers() {
      if (handlers === null) throw new Error("transport_not_subscribed");
      return handlers;
    },
    listenerCount(event?: RouterEvent) {
      if (event !== undefined) return listeners.get(event)?.size ?? 0;
      return [...listeners.values()].reduce(
        (count, registered) => count + registered.size,
        0,
      );
    },
    get page() {
      return page;
    },
    reloads,
    router: {
      on(event: RouterEvent, listener: RouterListener) {
        const registered = listeners.get(event) ?? new Set<RouterListener>();
        registered.add(listener);
        listeners.set(event, registered);
        return () => {
          registered.delete(listener);
        };
      },
      reload(options: (typeof reloads)[number]) {
        reloads.push(options);
      },
    },
    runNextTimer() {
      const entry = [...timers.entries()].sort(
        ([left], [right]) => left - right,
      )[0];
      if (entry === undefined) throw new Error("timer_missing");
      timers.delete(entry[0]);
      entry[1].callback();
      return entry[1].milliseconds;
    },
    setPage(value: typeof page) {
      page = value;
    },
    setTimer(callback: () => void, milliseconds: number) {
      const handle = nextTimer++;
      timers.set(handle, { callback, milliseconds });
      return handle;
    },
    get timerCount() {
      return timers.size;
    },
    finishVisit(visit: Visit) {
      for (const listener of listeners.get("finish") ?? []) {
        listener({ defaultPrevented: false, detail: { visit } });
      }
    },
    renderVisit(event: "error" | "success", visit: Visit) {
      for (const listener of listeners.get(event) ?? []) {
        listener({
          defaultPrevented: false,
          detail: { page, visitId: visit.id },
        });
      }
    },
    visit() {
      return this.beginVisit();
    },
    transport: {
      subscribe(_bindings: unknown, next: typeof handlers) {
        subscribedBindings = _bindings as object;
        handlers = next;
        return {
          close() {
            closed += 1;
          },
          documentInstance: "socket-document-1",
        };
      },
    },
  };
}

describe("Inertia live client core", () => {
  let connection: { close(): void } | null = null;

  afterEach(() => {
    connection?.close();
    connection = null;
  });

  it("subscribes topic names that match Object prototype properties", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    const bindings = JSON.parse(JSON.stringify(app.page.liveBindings));
    Object.defineProperty(bindings, "__proto__", {
      enumerable: true,
      value: bindings.notes,
    });
    app.setPage({ ...app.page, liveBindings: bindings });
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });
    assert.equal(Object.hasOwn(app.subscribedBindings, "__proto__"), true);
    app.handlers.onSignal({ topic: "__proto__", cursor: 11, props: ["stats"] });
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, ["stats"]);
  });

  it("subscribes every binding when a page has 33 topics", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    const bindings = Object.fromEntries(
      Array.from({ length: 33 }, (_, index) => [
        `topic-${index}`,
        { ...app.page.liveBindings.notes },
      ]),
    );
    app.setPage({ ...app.page, liveBindings: bindings });
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });
    assert.equal(Object.keys(app.subscribedBindings).length, 33);
    app.handlers.onSignal({ topic: "topic-32", cursor: 11, props: ["stats"] });
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, ["stats"]);
  });

  it("reloads every bound prop when one topic covers 257 props", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    const only = Array.from({ length: 257 }, (_, index) => `prop-${index}`);
    app.page.liveBindings.notes.only = only;
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });
    app.handlers.onSignal({ topic: "notes", cursor: 11, props: [] });
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, [...only].sort());
  });

  it("takes a reset baseline without requeueing a completed push batch", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });
    app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
    app.runNextTimer();
    app.page.liveBindings.notes.cursor = 0;
    app.reloads[0]?.onSuccess?.(app.page);
    app.reloads[0]?.onFinish?.();
    assert.equal(app.timerCount, 0);

    app.handlers.onSignal({
      topic: "notes",
      cursor: 1,
      props: ["recent_notes"],
    });
    assert.equal(app.timerCount, 1);
    app.handlers.onSignal({ topic: "notes", cursor: 1, props: ["stats"] });
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[1]?.only, ["recent_notes", "stats"]);
    app.page.liveBindings.notes.cursor = 1;
    app.reloads[1]?.onSuccess?.(app.page);
    app.reloads[1]?.onFinish?.();
    assert.equal(app.timerCount, 0);
    assert.equal(app.reloads.length, 2);
  });

  it("drops rendered cursors and coalesces a burst into one bounded partial reload", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      coalesceMilliseconds: 25,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    app.handlers.onSignal({ topic: "notes", cursor: 10, props: ["stats"] });
    assert.equal(app.timerCount, 0);

    app.handlers.onSignal({ topic: "notes", cursor: 11, props: [] });
    app.handlers.onSignal({ topic: "profile", cursor: 4, props: ["auth"] });
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["stats", "not_bound"],
    });
    assert.equal(app.timerCount, 1);
    assert.equal(app.runNextTimer(), 25);

    assert.equal(app.reloads.length, 1);
    assert.deepEqual(app.reloads[0]?.only, ["auth", "recent_notes", "stats"]);
    assert.deepEqual(app.reloads[0]?.headers, {
      "X-Socket-ID": "socket-document-1",
    });
    assert.deepEqual(app.visit().headers, {
      Existing: "kept",
      "X-Socket-ID": "socket-document-1",
    });
  });

  it("tracks rendered cursors per prop across partial-reload races", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
    app.runNextTimer();
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    assert.equal(app.timerCount, 0);

    const returned = {
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 12 },
      },
      props: {},
    };
    app.setPage(returned);
    app.reloads[0]?.onSuccess?.(returned);
    app.reloads[0]?.onFinish?.();
    assert.equal(app.timerCount, 1);
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    app.handlers.onSignal({ topic: "notes", cursor: 12, props: ["stats"] });
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[1]?.only, ["recent_notes"]);

    app.reloads[1]?.onSuccess?.({
      liveBindings: {
        ...returned.liveBindings,
        notes: { ...returned.liveBindings.notes, cursor: 12 },
      },
      props: {},
    });
    app.reloads[1]?.onFinish?.();
    app.handlers.onSignal({ topic: "notes", cursor: 12, props: ["stats"] });
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    assert.equal(app.timerCount, 0);
  });

  it("reconciles an external partial reload per rendered prop", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    const poll = app.beginVisit({ only: ["stats"] });
    app.page.liveBindings.notes.cursor = 20;
    app.renderVisit("success", poll);
    app.finishVisit(poll);

    app.handlers.onSignal({
      topic: "notes",
      cursor: 15,
      props: ["stats", "recent_notes"],
    });
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, ["recent_notes"]);
  });

  it("reconciles an external partial reload that renders errors", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    const poll = app.beginVisit({ only: ["stats"] });
    app.page.liveBindings.notes.cursor = 20;
    app.renderVisit("error", poll);
    app.finishVisit(poll);

    app.handlers.onSignal({ topic: "notes", cursor: 15, props: ["stats"] });
    assert.equal(app.timerCount, 0);
  });

  it("does not advance cursors when an external reload only finishes", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    const canceled = app.beginVisit({ only: ["stats"] });
    app.page.liveBindings.notes.cursor = 20;
    app.finishVisit(canceled);

    app.handlers.onSignal({ topic: "notes", cursor: 15, props: ["stats"] });
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, ["stats"]);
  });

  it("removes every Inertia lifecycle listener on close", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    assert.equal(app.listenerCount(), 4);
    connection.close();
    connection = null;
    assert.equal(app.listenerCount(), 0);
  });

  it("reloads the bound props on the transport polling fallback and cleans up on close", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      getPage: () => app.page,
      router: app.router,
      setTimer: app.setTimer,
      transport: app.transport,
    });

    const outcomes: string[] = [];
    app.handlers.onPoll("notes", (outcome) =>
      outcomes.push(`notes:${outcome}`),
    );
    app.handlers.onPoll("profile", (outcome) =>
      outcomes.push(`profile:${outcome}`),
    );
    assert.equal(app.timerCount, 1);
    app.runNextTimer();
    assert.deepEqual(app.reloads[0]?.only, ["auth", "recent_notes", "stats"]);
    app.reloads[0]?.onSuccess?.(app.page);
    app.reloads[0]?.onFinish?.();
    assert.deepEqual(outcomes, ["notes:succeeded", "profile:succeeded"]);

    app.handlers.onPoll("notes", (outcome) =>
      outcomes.push(`pending:${outcome}`),
    );

    connection.close();
    connection = null;
    assert.equal(app.closed, 1);
    assert.deepEqual(outcomes, [
      "notes:succeeded",
      "profile:succeeded",
      "pending:canceled",
    ]);
    assert.deepEqual(app.visit().headers, { Existing: "kept" });
    app.handlers.onSignal({ topic: "notes", cursor: 20, props: ["stats"] });
    assert.equal(app.timerCount, 0);
  });
});
