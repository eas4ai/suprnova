/**
 * The WebMCP lifecycle of a page's tools (AGENT-012, AGENT-014): when each
 * island's tools, and the page's server tools, are registered, the calls
 * each has pending, and the hold a refusal puts on an island's recovery
 * until those calls have their results. The page's server tools are one
 * more entry, keyed by an object of the document's own, so suspend,
 * resume and a page-level release cover them as they cover islands.
 *
 * A refusal or a navigation can end the page: the island reloads to
 * recover, or the document navigates away. A pending call whose page goes
 * first ends without a result, so the agent never hears of the refusal.
 * {@link AgentToolHost.release} answers every pending call of the island
 * first, unregisters the island's tools so no new call starts, and holds the
 * recovery long enough for the browser to hand the results to the agent;
 * {@link AgentToolHost.releaseAll} does the same for every island before a
 * reload or navigation of the whole page.
 */
import type {
  ModelContextClient,
  ModelContextLike,
  ModelContextToolResult,
  RegistrationFailure,
  WebMcpToolDefinition,
} from "./model-context.js";
import { registerAgentTools } from "./model-context.js";
import { agentToolError } from "./result.js";
import type { AgentToolDefinition } from "../islands/metadata.js";

/**
 * How long a recovery waits after the last pending result was returned.
 * The browser forwards a result to the agent asynchronously, and the page
 * cannot observe when it has: against Chromium's WebMCP, a reload within
 * about 10 ms of the result ended the call without it, and a reload 25 ms
 * or more after it delivered the result every time. The hold is ten times
 * that, and applies only when a call is pending: a refused click recovers
 * as it always has.
 */
export const AGENT_RESULT_HOLD_MS = 250;

/** What a call through a registration that has left the page resolves to. */
const LEFT = "The component's tools have left the page: call again once the page lists them.";

/** What the host registers for one island, or for the page's server tools. */
export interface AgentToolRegistration<Tool extends WebMcpToolDefinition = AgentToolDefinition> {
  readonly context: ModelContextLike;
  readonly tools: readonly Tool[];
  readonly execute: (
    tool: Tool,
    input: unknown,
    client?: ModelContextClient,
  ) => Promise<ModelContextToolResult>;
  readonly onFailure: (failure: RegistrationFailure) => void;
}

/**
 * A registration with its tool type put away: it registers its tools,
 * answering a call only while `current` holds, and returns the abort.
 */
type ErasedRegistration = (current: () => boolean) => VoidFunction;

/** The timer the hold waits on. */
export interface AgentHostScheduler {
  timeout(callback: () => void, delay: number): unknown;
}

/** One call of an island's tool that has no result yet. */
export class PendingAgentCall {
  readonly #settle: (result: ModelContextToolResult) => void;
  readonly #pending: Set<PendingAgentCall>;
  #settled = false;

  constructor(settle: (result: ModelContextToolResult) => void, pending: Set<PendingAgentCall>) {
    this.#settle = settle;
    this.#pending = pending;
    pending.add(this);
  }

  /** Answer the call with `result`, unless it already has an answer. */
  settle(result: ModelContextToolResult): boolean {
    if (this.#settled) return false;
    this.#settled = true;
    this.#pending.delete(this);
    this.#settle(result);
    return true;
  }

  /** The call ended another way, its agent's abort above all: forget it. */
  withdraw(): void {
    this.#settled = true;
    this.#pending.delete(this);
  }
}

interface Entry {
  /** The island's registration now, or `null` when it cannot register. */
  readonly registrar: () => ErasedRegistration | null;
  /** Aborts the current registration; `null` while none is live. */
  abort: VoidFunction | null;
  /** The current registration; a call through any other is refused. */
  token: object | null;
  readonly pending: Set<PendingAgentCall>;
}

export class AgentToolHost<Island extends object> {
  readonly #entries = new Map<Island, Entry>();
  readonly #scheduler: AgentHostScheduler;
  #suspended = false;

  constructor(scheduler: AgentHostScheduler) {
    this.#scheduler = scheduler;
  }

  /**
   * Register the tools `registrar` gives for `island` now, and again on
   * each resume or restore. `registrar` builds the registration each time,
   * so every call it accepts is bound to the island's current connection.
   */
  connect<Tool extends WebMcpToolDefinition>(
    island: Island,
    registrar: () => AgentToolRegistration<Tool> | null,
  ): void {
    if (this.#entries.has(island)) return;
    const erased = (): ErasedRegistration | null => {
      const registration = registrar();
      if (registration === null) return null;
      return (current) =>
        registerAgentTools(
          registration.context,
          registration.tools,
          (tool, input, client) =>
            current()
              ? registration.execute(tool, input, client)
              : Promise.resolve(agentToolError(LEFT)),
          registration.onFailure,
        );
    };
    const entry: Entry = { registrar: erased, abort: null, token: null, pending: new Set() };
    this.#entries.set(island, entry);
    if (!this.#suspended) this.#register(entry);
  }

  /** The island left the page: its tools leave with it. */
  disconnect(island: Island): void {
    const entry = this.#entries.get(island);
    if (entry === undefined) return;
    this.#entries.delete(island);
    this.#unregister(entry);
  }

  /** The page entered the back/forward cache: every tool leaves the agent's view. */
  suspend(): void {
    this.#suspended = true;
    for (const entry of this.#entries.values()) this.#unregister(entry);
  }

  /** The page came back: every island still connected registers again. */
  resume(): void {
    this.#suspended = false;
    for (const entry of this.#entries.values()) this.#register(entry);
  }

  /**
   * Record a call of `island`'s tool as pending until it is settled, by the
   * caller with its own result or by {@link release}. `null` when the island
   * is not connected.
   */
  track(island: Island, settle: (result: ModelContextToolResult) => void): PendingAgentCall | null {
    const entry = this.#entries.get(island);
    return entry === undefined ? null : new PendingAgentCall(settle, entry.pending);
  }

  /**
   * `island` alone is about to stop: unregister its tools at once, answer
   * every pending call with `answer`, then hand each to `after`, and
   * resolve once the browser has had {@link AGENT_RESULT_HOLD_MS} to pass
   * those results to the agent. With no call pending it resolves at once.
   */
  async release(
    island: Island,
    answer: (call: PendingAgentCall, island: Island) => ModelContextToolResult,
    after: (call: PendingAgentCall, island: Island) => void = () => undefined,
  ): Promise<void> {
    const entry = this.#entries.get(island);
    if (entry !== undefined && this.#releaseEntry(island, entry, answer, after)) {
      await this.#hold();
    }
  }

  /**
   * The whole page is about to reload or navigate: every island's tools
   * leave at once, every island's pending calls are answered, and the
   * promise resolves after the hold when any island had one. A reload for
   * one island ends every island's calls, so all of them hear first.
   */
  async releaseAll(
    answer: (call: PendingAgentCall, island: Island) => ModelContextToolResult,
    after: (call: PendingAgentCall, island: Island) => void = () => undefined,
  ): Promise<void> {
    let pending = false;
    for (const [island, entry] of this.#entries) {
      if (this.#releaseEntry(island, entry, answer, after)) pending = true;
    }
    if (pending) await this.#hold();
  }

  /** `island` stayed on the page after a release: register its tools again. */
  restore(island: Island): void {
    const entry = this.#entries.get(island);
    if (entry !== undefined && !this.#suspended) this.#register(entry);
  }

  /** The page stayed after a release of every island: register them all again. */
  restoreAll(): void {
    if (this.#suspended) return;
    for (const entry of this.#entries.values()) this.#register(entry);
  }

  /** Unregister `entry` and answer its pending calls; whether it had any. */
  #releaseEntry(
    island: Island,
    entry: Entry,
    answer: (call: PendingAgentCall, island: Island) => ModelContextToolResult,
    after: (call: PendingAgentCall, island: Island) => void,
  ): boolean {
    this.#unregister(entry);
    const calls = [...entry.pending];
    for (const call of calls) {
      call.settle(answer(call, island));
      after(call, island);
    }
    return calls.length > 0;
  }

  #hold(): Promise<void> {
    return new Promise<void>((resolve) => {
      this.#scheduler.timeout(resolve, AGENT_RESULT_HOLD_MS);
    });
  }

  #register(entry: Entry): void {
    if (entry.abort !== null) return;
    const register = entry.registrar();
    if (register === null) return;
    const token = {};
    entry.token = token;
    entry.abort = register(() => entry.token === token);
  }

  #unregister(entry: Entry): void {
    entry.token = null;
    const abort = entry.abort;
    entry.abort = null;
    abort?.();
  }
}
