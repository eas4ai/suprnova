import { afterEach, describe, expect, it } from "bun:test";

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
  const source = await Bun.file(url).text();
  const javascript = new Bun.Transpiler({
    loader: "ts",
    target: "browser",
  }).transformSync(source);
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
    props: { stats: { notes: 0 }, recent_notes: [], auth: {} },
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

  it.skipIf(!Bun.env.SIGNAL_LIVE_BASELINE_PAGE)(
    "accepts the committed signal newer than the actual Rust loader page",
    async () => {
      const { startInertiaLive } = await loadCore();
      const app = fixture();
      const page = await Bun.file(Bun.env.SIGNAL_LIVE_BASELINE_PAGE!).json();
      expect(page.liveBindings["signal.render"].cursor).toBe(0);
      expect(page.props.posts).toEqual(["old query value"]);
      app.setPage(page);
      connection = startInertiaLive({
        clearTimer: app.clearTimer,
        setTimer: app.setTimer,
        router: app.router,
        transport: app.transport,
        getPage: () => app.page,
      });
      app.handlers.onSignal({
        topic: "signal.render",
        cursor: 1,
        props: ["posts"],
      });
      app.runNextTimer();
      expect(app.reloads[0]?.only).toEqual(["posts"]);
      app.reloads[0]?.onSuccess?.({
        ...page,
        liveBindings: {
          "signal.render": { ...page.liveBindings["signal.render"], cursor: 1 },
        },
        props: { posts: ["committed new value"] },
      });
      app.reloads[0]?.onFinish?.();
      app.handlers.onSignal({
        topic: "signal.render",
        cursor: 1,
        props: ["posts"],
      });
      expect(app.timerCount).toBe(0);
    },
  );

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
    expect(Object.hasOwn(app.subscribedBindings, "__proto__")).toBe(true);
    app.handlers.onSignal({ topic: "__proto__", cursor: 11, props: ["stats"] });
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["stats"]);
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
    expect(Object.keys(app.subscribedBindings).length).toBe(33);
    app.handlers.onSignal({ topic: "topic-32", cursor: 11, props: ["stats"] });
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["stats"]);
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
    expect(app.reloads[0]?.only).toEqual([...only].sort());
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
    expect(app.timerCount).toBe(0);

    app.handlers.onSignal({
      topic: "notes",
      cursor: 1,
      props: ["recent_notes"],
    });
    expect(app.timerCount).toBe(1);
    app.handlers.onSignal({ topic: "notes", cursor: 1, props: ["stats"] });
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[1]?.only).toEqual(["recent_notes", "stats"]);
    app.page.liveBindings.notes.cursor = 1;
    app.reloads[1]?.onSuccess?.(app.page);
    app.reloads[1]?.onFinish?.();
    expect(app.timerCount).toBe(0);
    expect(app.reloads.length).toBe(2);
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
    expect(app.timerCount).toBe(0);

    app.handlers.onSignal({ topic: "notes", cursor: 11, props: [] });
    app.handlers.onSignal({ topic: "profile", cursor: 4, props: ["auth"] });
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["stats", "not_bound"],
    });
    expect(app.timerCount).toBe(1);
    expect(app.runNextTimer()).toBe(25);

    expect(app.reloads.length).toBe(1);
    expect(app.reloads[0]?.only).toEqual(["auth", "recent_notes", "stats"]);
    expect(app.reloads[0]?.headers).toEqual({
      "X-Socket-ID": "socket-document-1",
    });
    expect(app.visit().headers).toEqual({
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
    expect(app.timerCount).toBe(0);

    const returned = {
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 12 },
      },
      props: { stats: { notes: 1 } },
    };
    app.setPage(returned);
    app.reloads[0]?.onSuccess?.(returned);
    app.reloads[0]?.onFinish?.();
    expect(app.timerCount).toBe(1);
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    app.handlers.onSignal({ topic: "notes", cursor: 12, props: ["stats"] });
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[1]?.only).toEqual(["recent_notes"]);

    app.reloads[1]?.onSuccess?.({
      liveBindings: {
        ...returned.liveBindings,
        notes: { ...returned.liveBindings.notes, cursor: 12 },
      },
      props: { recent_notes: [] },
    });
    app.reloads[1]?.onFinish?.();
    app.handlers.onSignal({ topic: "notes", cursor: 12, props: ["stats"] });
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    expect(app.timerCount).toBe(0);
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
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["recent_notes"]);
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
    expect(app.timerCount).toBe(0);
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
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["stats"]);
  });

  it("keeps rescued requested props invalidated while advancing rendered props", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      setTimer: app.setTimer,
      router: app.router,
      transport: app.transport,
      getPage: () => app.page,
    });
    app.handlers.onSignal({
      topic: "notes",
      cursor: 11,
      props: ["stats", "recent_notes"],
    });
    app.runNextTimer();
    app.reloads[0]?.onSuccess?.({
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 11 },
      },
      props: { stats: { notes: 1 }, recent_notes: ["old merged value"] },
      rescuedProps: ["recent_notes"],
    });
    app.reloads[0]?.onFinish?.();
    app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[1]?.only).toEqual(["recent_notes"]);
    app.reloads[1]?.onSuccess?.({
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 11 },
      },
      props: { recent_notes: ["fresh"] },
    });
    app.reloads[1]?.onFinish?.();
    app.handlers.onSignal({
      topic: "notes",
      cursor: 11,
      props: ["stats", "recent_notes"],
    });
    expect(app.timerCount).toBe(0);
  });

  it("does not advance an omitted deferred prop on an external reload", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      setTimer: app.setTimer,
      router: app.router,
      transport: app.transport,
      getPage: () => app.page,
    });
    const visit = app.beginVisit({ only: ["stats", "recent_notes"] });
    app.setPage({
      ...app.page,
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 20 },
      },
      props: { stats: { notes: 2 } },
      deferredProps: { later: ["recent_notes"] },
    });
    app.renderVisit("success", visit);
    app.finishVisit(visit);
    app.handlers.onSignal({
      topic: "notes",
      cursor: 15,
      props: ["stats", "recent_notes"],
    });
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["recent_notes"]);
  });

  for (const outcome of ["failed", "canceled"]) {
    it(`retries a ${outcome} reload without another publication`, async () => {
      const { startInertiaLive } = await loadCore();
      const app = fixture();
      connection = startInertiaLive({
        clearTimer: app.clearTimer,
        setTimer: app.setTimer,
        router: app.router,
        transport: app.transport,
        getPage: () => app.page,
      });
      app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
      expect(app.runNextTimer()).toBe(25);
      app.reloads[0]?.onFinish?.();
      expect(app.timerCount).toBe(1);
      expect(app.runNextTimer()).toBe(1_000);
      expect(app.reloads[1]?.only).toEqual(["stats"]);
      app.reloads[1]?.onSuccess?.({
        liveBindings: {
          ...app.page.liveBindings,
          notes: { ...app.page.liveBindings.notes, cursor: 11 },
        },
        props: { stats: { notes: 1 } },
      });
      app.reloads[1]?.onFinish?.();
      expect(app.timerCount).toBe(0);
      app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
      expect(app.timerCount).toBe(0);
    });
  }

  it("caps retry backoff, merges concurrent invalidations and resets after success", async () => {
    const { startInertiaLive } = await loadCore();
    const app = fixture();
    connection = startInertiaLive({
      clearTimer: app.clearTimer,
      setTimer: app.setTimer,
      router: app.router,
      transport: app.transport,
      getPage: () => app.page,
    });
    app.handlers.onSignal({ topic: "notes", cursor: 11, props: ["stats"] });
    app.runNextTimer();
    app.handlers.onSignal({
      topic: "notes",
      cursor: 12,
      props: ["recent_notes"],
    });
    for (const [index, milliseconds] of [
      1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000,
    ].entries()) {
      app.reloads[index]?.onFinish?.();
      expect(app.runNextTimer()).toBe(milliseconds);
      expect(app.reloads[index + 1]?.only).toEqual(["recent_notes", "stats"]);
    }
    app.reloads[7]?.onSuccess?.({
      liveBindings: {
        ...app.page.liveBindings,
        notes: { ...app.page.liveBindings.notes, cursor: 12 },
      },
      props: { stats: { notes: 1 }, recent_notes: [] },
    });
    app.reloads[7]?.onFinish?.();
    expect(app.timerCount).toBe(0);
    app.handlers.onSignal({ topic: "notes", cursor: 13, props: ["stats"] });
    expect(app.runNextTimer()).toBe(25);
    app.reloads[8]?.onFinish?.();
    expect(app.runNextTimer()).toBe(1_000);
    connection.close();
    connection = null;
    expect(app.timerCount).toBe(0);
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

    expect(app.listenerCount()).toBe(4);
    connection.close();
    connection = null;
    expect(app.listenerCount()).toBe(0);
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
    expect(app.timerCount).toBe(1);
    app.runNextTimer();
    expect(app.reloads[0]?.only).toEqual(["auth", "recent_notes", "stats"]);
    app.reloads[0]?.onSuccess?.(app.page);
    app.reloads[0]?.onFinish?.();
    expect(outcomes).toEqual(["notes:succeeded", "profile:succeeded"]);

    app.handlers.onPoll("notes", (outcome) =>
      outcomes.push(`pending:${outcome}`),
    );

    connection.close();
    connection = null;
    expect(app.closed).toBe(1);
    expect(outcomes).toEqual([
      "notes:succeeded",
      "profile:succeeded",
      "pending:canceled",
    ]);
    expect(app.visit().headers).toEqual({ Existing: "kept" });
    app.handlers.onSignal({ topic: "notes", cursor: 20, props: ["stats"] });
    expect(app.timerCount).toBe(0);
  });
});
