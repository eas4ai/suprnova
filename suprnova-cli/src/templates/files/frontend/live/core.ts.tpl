/** The framework-owned identity used to issue one Live subscription. */
export type InertiaLiveIsland = {
  component: string;
  document_key: string;
  slot: string;
};

/** One topic binding rendered with an Inertia page. */
export type InertiaLiveBinding = {
  cursor: number;
  generation: number;
  island: InertiaLiveIsland;
  only: string[];
  stream: string;
};

export type InertiaLiveBindings = Record<string, InertiaLiveBinding>;

export type InertiaLiveSignal = {
  cursor: number;
  generation: number;
  props: readonly string[];
  topic: string;
};

export type InertiaLiveState =
  | "disconnected"
  | "connecting"
  | "current"
  | "degraded"
  | "reconnecting"
  | "closed";

export type InertiaLiveTransportCallbacks = {
  onPoll: (topic: string, completion?: InertiaLivePollCompletion) => void;
  onSignal: (signal: InertiaLiveSignal) => void;
  onState: (topic: string, state: InertiaLiveState) => void;
};

export type InertiaLivePollCompletion = (
  outcome: "succeeded" | "failed" | "canceled" | "retired",
) => void;

export type InertiaLiveTransportConnection = {
  close: () => void;
  documentInstance: string;
};

export type InertiaLiveTransport = {
  subscribe: (
    bindings: InertiaLiveBindings,
    callbacks: InertiaLiveTransportCallbacks,
  ) => InertiaLiveTransportConnection;
};

export type InertiaLivePage<
  Props extends Record<string, unknown> = Record<string, unknown>,
> = {
  liveBindings?: InertiaLiveBindings;
  rescuedProps?: readonly string[];
  deferredProps?: Record<string, readonly string[]>;
  props: Props;
};

type ReloadOptions = {
  headers: Record<string, string>;
  only: string[];
  onFinish: () => void;
  onSuccess: (page: InertiaLivePage) => void;
};

type InertiaLiveVisit = {
  except: readonly string[];
  headers?: Record<string, string>;
  id: string;
  only: readonly string[];
  reset: readonly string[];
};

type InertiaLiveRouterEvents = {
  before: {
    defaultPrevented: boolean;
    detail: { visit: InertiaLiveVisit };
  };
  error: {
    detail: { page?: InertiaLivePage; visitId?: string };
  };
  finish: {
    detail: { visit: InertiaLiveVisit };
  };
  success: {
    detail: { page: InertiaLivePage; visitId?: string };
  };
};

export type InertiaLiveRouter = {
  on: <Event extends keyof InertiaLiveRouterEvents>(
    event: Event,
    listener: (event: InertiaLiveRouterEvents[Event]) => void,
  ) => () => void;
  reload: (options: ReloadOptions) => void;
};

type TimerHandle = ReturnType<typeof setTimeout>;

export type InertiaLiveOptions = {
  clearTimer?: (handle: TimerHandle) => void;
  coalesceMilliseconds?: number;
  getPage: () => InertiaLivePage;
  onState?: (topic: string, state: InertiaLiveState) => void;
  router: InertiaLiveRouter;
  setTimer?: (callback: () => void, milliseconds: number) => TimerHandle;
  transport: InertiaLiveTransport;
};

export type InertiaLiveConnection = {
  close: () => void;
};

type TopicPosition = { generation: number; cursor: number };

function covers(
  rendered: TopicPosition | undefined,
  change: TopicPosition,
): boolean {
  return (
    rendered !== undefined &&
    rendered.generation === change.generation &&
    rendered.cursor >= change.cursor
  );
}

type PendingTopic = {
  completions: Set<InertiaLivePollCompletion>;
  props: Map<string, TopicPosition | null>;
};

type VisitSelection = Pick<InertiaLiveVisit, "except" | "only" | "reset">;

const DEFAULT_COALESCE_MILLISECONDS = 25;
const MAX_COALESCE_MILLISECONDS = 1_000;
const CONTRACT_NAME = /^[A-Za-z0-9._:/-]+$/u;

function contractName(value: unknown, maximum: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= maximum &&
    CONTRACT_NAME.test(value)
  );
}

function validBinding(value: unknown): value is InertiaLiveBinding {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    return false;
  const binding = value as Partial<InertiaLiveBinding>;
  const island = binding.island;
  return (
    Number.isSafeInteger(binding.generation) &&
    (binding.generation ?? 0) > 0 &&
    Number.isSafeInteger(binding.cursor) &&
    (binding.cursor ?? -1) >= 0 &&
    Array.isArray(binding.only) &&
    binding.only.length > 0 &&
    binding.only.every((prop) => typeof prop === "string" && prop.length > 0) &&
    new Set(binding.only).size === binding.only.length &&
    contractName(binding.stream, 128) &&
    island !== null &&
    typeof island === "object" &&
    contractName(island.component, 256) &&
    contractName(island.document_key, 256) &&
    contractName(island.slot, 256)
  );
}

function pageBindings(page: InertiaLivePage): InertiaLiveBindings {
  const value = page.liveBindings;
  if (value === null || typeof value !== "object" || Array.isArray(value))
    return {};
  const entries = Object.entries(value);
  const bindings = Object.create(null) as InertiaLiveBindings;
  for (const [topic, binding] of entries) {
    if (
      !contractName(topic, 256) ||
      topic
        .split("/")
        .some(
          (segment) =>
            segment.length === 0 || segment === "." || segment === "..",
        ) ||
      !validBinding(binding)
    ) {
      return {};
    }
    bindings[topic] = binding;
  }
  return bindings;
}

function transportShape(bindings: InertiaLiveBindings): string {
  return Object.entries(bindings)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(
      ([topic, binding]) =>
        `${topic}\u0000${binding.stream}\u0000${binding.island.component}\u0000${binding.island.slot}\u0000${binding.island.document_key}`,
    )
    .join("\u0001");
}

function reloadedProps(
  page: InertiaLivePage,
  selection: VisitSelection,
): string[] {
  const selected = new Set([...selection.only, ...selection.reset]);
  const candidates =
    selected.size > 0
      ? selected
      : new Set(
          Object.values(pageBindings(page)).flatMap((binding) => binding.only),
        );
  const excluded = new Set(selection.except);
  return [...candidates].filter((prop) => !excluded.has(prop));
}

function coalesceWindow(value: number | undefined): number {
  if (value === undefined) return DEFAULT_COALESCE_MILLISECONDS;
  if (
    !Number.isSafeInteger(value) ||
    value < 0 ||
    value > MAX_COALESCE_MILLISECONDS
  ) {
    throw new Error("inertia_live_coalesce_invalid");
  }
  return value;
}

function currentBrowserTransport(): InertiaLiveTransport | null {
  let candidate: unknown;
  try {
    candidate = Reflect.get(globalThis, "SuprnovaLiveAsync");
  } catch {
    return null;
  }
  if (candidate === null || typeof candidate !== "object") return null;
  const subscribeTopics = Reflect.get(candidate, "subscribeTopics");
  if (typeof subscribeTopics !== "function") return null;
  return Object.freeze({
    subscribe(bindings, callbacks) {
      return Reflect.apply(subscribeTopics, candidate, [
        bindings,
        callbacks,
      ]) as InertiaLiveTransportConnection;
    },
  });
}

/** The existing Live async artifact, when the framework loaded it for this page. */
export function browserLiveTransport(): InertiaLiveTransport | null {
  return currentBrowserTransport();
}

/**
 * Connect one mounted Inertia page to Live. Signals inside the coalescing
 * window become one partial reload. Closing the returned connection releases
 * its timer, memberships and physical transport references.
 */
export function startInertiaLive(
  options: InertiaLiveOptions,
): InertiaLiveConnection {
  const clear = options.clearTimer ?? clearTimeout;
  const schedule = options.setTimer ?? setTimeout;
  const delay = coalesceWindow(options.coalesceMilliseconds);
  let bindings = pageBindings(options.getPage());
  let bindingShape = transportShape(bindings);
  const renderedPositions = new Map<string, Map<string, TopicPosition>>(
    Object.entries(bindings).map(([topic, binding]) => [
      topic,
      new Map(
        binding.only.map((prop) => [
          prop,
          { generation: binding.generation, cursor: binding.cursor },
        ]),
      ),
    ]),
  );
  let closed = false;
  let timer: TimerHandle | null = null;
  let reloading = false;
  let retryAttempt = 0;
  let retryMilliseconds = 0;
  let transport: InertiaLiveTransportConnection | null = null;
  const activeCompletions = new Set<InertiaLivePollCompletion>();
  const pending = new Map<string, PendingTopic>();
  const visits = new Map<string, VisitSelection>();

  const queue = (
    topic: string,
    props: readonly string[],
    position: TopicPosition | null,
    completion?: InertiaLivePollCompletion,
  ) => {
    if (closed) return;
    const binding = bindings[topic];
    if (binding === undefined) return;
    if (
      position !== null &&
      (!Number.isSafeInteger(position.cursor) ||
        position.cursor < 0 ||
        !Number.isSafeInteger(position.generation) ||
        position.generation < 1)
    )
      return;
    const rendered = renderedPositions.get(topic);
    const reset =
      position !== null &&
      (position.generation !== binding.generation ||
        binding.only.some((prop) => {
          const baseline = rendered?.get(prop);
          return (
            baseline === undefined ||
            baseline.generation !== position.generation ||
            baseline.cursor < 0
          );
        }));
    const allowed = new Set(binding.only);
    const selected =
      props.length === 0 || reset
        ? binding.only
        : props.filter((prop) => allowed.has(prop));
    if (selected.length === 0) return;
    const entry = pending.get(topic) ?? {
      completions: new Set<InertiaLivePollCompletion>(),
      props: new Map<string, TopicPosition | null>(),
    };
    for (const prop of selected) {
      const baseline = rendered?.get(prop);
      if (
        position !== null &&
        baseline?.generation === position.generation &&
        position.cursor <= baseline.cursor
      )
        continue;
      const queued = entry.props.get(prop);
      if (
        !entry.props.has(prop) ||
        (position !== null &&
          (queued === null ||
            queued === undefined ||
            queued.generation !== position.generation ||
            position.cursor > queued.cursor))
      ) {
        entry.props.set(prop, position);
      }
    }
    if (entry.props.size === 0) return;
    if (completion !== undefined) entry.completions.add(completion);
    pending.set(topic, entry);
    arm();
  };

  const callbacks: InertiaLiveTransportCallbacks = {
    onPoll(topic, completion) {
      const binding = bindings[topic];
      if (binding !== undefined) queue(topic, binding.only, null, completion);
      else completion?.("retired");
    },
    onSignal(signal) {
      if (
        signal === null ||
        typeof signal !== "object" ||
        typeof signal.topic !== "string" ||
        !Array.isArray(signal.props)
      ) {
        return;
      }
      queue(signal.topic, signal.props, {
        generation: signal.generation,
        cursor: signal.cursor,
      });
    },
    onState(topic, state) {
      options.onState?.(topic, state);
    },
  };

  const connect = () => {
    transport?.close();
    transport = null;
    if (closed || Object.keys(bindings).length === 0) return;
    transport = options.transport.subscribe(bindings, callbacks);
  };

  const reconcile = (
    page: InertiaLivePage,
    reloadedProps: readonly string[],
  ) => {
    const next = pageBindings(page);
    const nextShape = transportShape(next);
    bindings = next;
    if (nextShape !== bindingShape) {
      bindingShape = nextShape;
      connect();
    }
    const omitted = new Set([
      ...(page.rescuedProps ?? []),
      ...Object.values(page.deferredProps ?? {}).flat(),
    ]);
    const reloaded = new Set(
      reloadedProps.filter(
        (prop) =>
          Object.prototype.hasOwnProperty.call(page.props, prop) &&
          !omitted.has(prop),
      ),
    );
    for (const topic of [...renderedPositions.keys()]) {
      if (bindings[topic] === undefined) renderedPositions.delete(topic);
    }
    for (const [topic, binding] of Object.entries(bindings)) {
      const rendered =
        renderedPositions.get(topic) ?? new Map<string, TopicPosition>();
      const bound = new Set(binding.only);
      for (const prop of [...rendered.keys()]) {
        if (!bound.has(prop)) rendered.delete(prop);
      }
      for (const prop of binding.only) {
        if (reloaded.has(prop)) {
          rendered.set(prop, {
            generation: binding.generation,
            cursor: binding.cursor,
          });
        } else if (
          !rendered.has(prop) ||
          rendered.get(prop)?.generation !== binding.generation ||
          (rendered.get(prop)?.cursor ?? -1) > binding.cursor
        ) {
          // A lower authority baseline makes unreloaded prop cursors incomparable.
          rendered.set(prop, { generation: binding.generation, cursor: -1 });
        }
      }
      renderedPositions.set(topic, rendered);
    }
    for (const [topic, entry] of pending) {
      const binding = bindings[topic];
      if (binding === undefined) {
        for (const complete of entry.completions) complete("retired");
        pending.delete(topic);
        continue;
      }
      const bound = new Set(binding.only);
      const rendered = renderedPositions.get(topic);
      for (const [prop, position] of entry.props) {
        if (
          !bound.has(prop) ||
          (reloaded.has(prop) &&
            (position === null || covers(rendered?.get(prop), position)))
        ) {
          entry.props.delete(prop);
        }
      }
      if (entry.props.size === 0) {
        for (const complete of entry.completions) complete("succeeded");
        pending.delete(topic);
      }
    }
    if (pending.size === 0 && timer !== null) {
      clear(timer);
      timer = null;
    }
    return reloaded;
  };

  const flush = () => {
    timer = null;
    if (closed || reloading || pending.size === 0) return;
    const batch = new Map(pending);
    pending.clear();
    const only = [
      ...new Set(
        [...batch.values()].flatMap((entry) => [...entry.props.keys()]),
      ),
    ].sort();
    if (only.length === 0) return;
    for (const entry of batch.values()) {
      for (const complete of entry.completions) activeCompletions.add(complete);
    }
    const completeBatch = (outcome: "succeeded" | "failed") => {
      for (const entry of batch.values()) {
        for (const complete of entry.completions) {
          if (activeCompletions.delete(complete)) complete(outcome);
        }
      }
    };
    reloading = true;
    let succeeded = false;
    let incomplete = false;
    let finished = false;
    const restoreBatch = (rendered = new Set<string>()) => {
      let restored = false;
      for (const [topic, entry] of batch) {
        for (const [prop, position] of entry.props) {
          if (
            rendered.has(prop) &&
            (position === null ||
              renderedPositions.get(topic)?.get(prop)?.generation !==
                position.generation ||
              covers(renderedPositions.get(topic)?.get(prop), position))
          )
            continue;
          queue(topic, [prop], position);
          restored ||= pending.get(topic)?.props.has(prop) ?? false;
        }
      }
      return restored;
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      if (closed) {
        reloading = false;
        return;
      }
      if (!succeeded) {
        completeBatch("failed");
        restoreBatch();
      }
      if ((!succeeded || incomplete) && pending.size > 0) {
        retryMilliseconds = Math.min(1_000 * 2 ** retryAttempt, 30_000);
        retryAttempt = Math.min(retryAttempt + 1, 5);
      } else {
        retryAttempt = 0;
        retryMilliseconds = 0;
      }
      reloading = false;
      arm();
    };
    const headers: Record<string, string> = {};
    if (transport?.documentInstance) {
      headers["X-Socket-ID"] = transport.documentInstance;
    }
    try {
      options.router.reload({
        headers,
        only,
        onFinish: finish,
        onSuccess(page) {
          if (closed || finished) return;
          succeeded = true;
          const rendered = reconcile(page, only);
          incomplete = restoreBatch(rendered);
          completeBatch(incomplete ? "failed" : "succeeded");
        },
      });
    } catch {
      finish();
    }
  };

  function arm() {
    if (closed || reloading || timer !== null || pending.size === 0) return;
    timer = schedule(flush, retryMilliseconds || delay);
  }

  const removeLifecycleListeners = [
    options.router.on("before", (event) => {
      const visit = event.detail.visit;
      if (!event.defaultPrevented) {
        visits.set(visit.id, {
          except: [...visit.except],
          only: [...visit.only],
          reset: [...visit.reset],
        });
        queueMicrotask(() => {
          if (event.defaultPrevented) visits.delete(visit.id);
        });
      }

      const documentInstance = transport?.documentInstance;
      if (documentInstance === undefined || documentInstance.length === 0)
        return;
      const headers = visit.headers ?? {};
      headers["X-Socket-ID"] = documentInstance;
      visit.headers = headers;
    }),
    options.router.on("success", (event) => {
      const visitId = event.detail.visitId;
      if (visitId === undefined) return;
      const selection = visits.get(visitId);
      if (selection === undefined) return;
      visits.delete(visitId);
      reconcile(event.detail.page, reloadedProps(event.detail.page, selection));
    }),
    options.router.on("error", (event) => {
      const { page, visitId } = event.detail;
      if (page === undefined || visitId === undefined) return;
      const selection = visits.get(visitId);
      if (selection === undefined) return;
      visits.delete(visitId);
      reconcile(page, reloadedProps(page, selection));
    }),
    options.router.on("finish", (event) => {
      visits.delete(event.detail.visit.id);
    }),
  ];

  try {
    connect();
  } catch {
    transport = null;
  }

  return Object.freeze({
    close() {
      if (closed) return;
      closed = true;
      if (timer !== null) clear(timer);
      timer = null;
      for (const entry of pending.values()) {
        for (const complete of entry.completions) complete("canceled");
      }
      pending.clear();
      for (const complete of activeCompletions) complete("canceled");
      activeCompletions.clear();
      visits.clear();
      for (const removeListener of removeLifecycleListeners) removeListener();
      transport?.close();
      transport = null;
    },
  });
}
