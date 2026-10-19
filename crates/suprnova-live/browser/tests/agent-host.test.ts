import { describe, expect, it } from "vitest";

import { AGENT_RESULT_HOLD_MS, AgentToolHost } from "../src/agents/host.js";
import type {
  ModelContextLike,
  ModelContextTool,
  ModelContextToolResult,
} from "../src/agents/model-context.js";
import { agentToolError } from "../src/agents/result.js";
import type { AgentToolDefinition } from "../src/islands/metadata.js";

const TOOL: AgentToolDefinition = Object.freeze({
  action: "increment",
  name: "counter.increment.main",
  description: "Add one to the counter.",
  inputSchema: Object.freeze({ type: "object" }),
  annotations: Object.freeze({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
  }),
});

/** A model context that records what the page registers. */
class FakeModelContext implements ModelContextLike {
  readonly registered: { tool: ModelContextTool; signal: AbortSignal | undefined }[] = [];

  registerTool(
    tool: ModelContextTool,
    options?: Readonly<{ signal?: AbortSignal }>,
  ): Promise<void> {
    this.registered.push({ tool, signal: options?.signal });
    return Promise.resolve();
  }

  /** The tools registered and not aborted, by name. */
  live(): string[] {
    return this.registered
      .filter((entry) => entry.signal?.aborted !== true)
      .map((entry) => entry.tool.name);
  }

  /** Call the most recent registration of `name` as WebMCP would. */
  call(name: string, input: unknown = {}): Promise<ModelContextToolResult> {
    const entries = this.registered.filter((candidate) => candidate.tool.name === name);
    const entry = entries[entries.length - 1];
    if (entry === undefined) throw new Error(`no registration of ${name}`);
    return entry.tool.execute(input);
  }
}

/** A scheduler whose timeouts run only when the test says. */
class ManualScheduler {
  readonly timeouts: { callback: () => void; delay: number }[] = [];

  timeout(callback: () => void, delay: number): number {
    this.timeouts.push({ callback, delay });
    return this.timeouts.length;
  }

  runAll(): void {
    for (const { callback } of this.timeouts.splice(0)) callback();
  }
}

/** Every pending promise reaction runs. */
async function flush(): Promise<void> {
  for (let turn = 0; turn < 10; turn += 1) await Promise.resolve();
}

function settledState<T>(promise: Promise<T>): { settled: boolean; value?: T } {
  const state: { settled: boolean; value?: T } = { settled: false };
  void promise.then((value) => {
    state.settled = true;
    state.value = value;
  });
  return state;
}

/**
 * A host with one island whose calls stay pending until the test answers
 * them through the host, as the document runtime's scheduler does.
 */
function hostWithIsland() {
  const context = new FakeModelContext();
  const scheduler = new ManualScheduler();
  const host = new AgentToolHost<object>(scheduler);
  const island = {};
  const scheduled: unknown[] = [];
  host.connect(island, () => ({
    context,
    tools: [TOOL],
    execute: (_tool: AgentToolDefinition, input: unknown) => {
      scheduled.push(input);
      return new Promise<ModelContextToolResult>((resolve) => {
        host.track(island, resolve);
      });
    },
    onFailure: () => undefined,
  }));
  return { context, scheduler, host, island, scheduled };
}

describe("the page's agent tool islands", () => {
  it("agent_012 registers an island's tools when it connects", () => {
    const { context } = hostWithIsland();
    expect(context.live()).toEqual(["counter.increment.main"]);
  });

  it("agent_012 a refusal answers the pending call before the island recovers", async () => {
    const { context, scheduler, host, island } = hostWithIsland();
    const pending = settledState(context.call("counter.increment.main"));
    await flush();
    const order: string[] = [];
    const refused = agentToolError("The action did not run.");
    const released = host.release(island, () => refused).then(() => order.push("recovery"));
    // The tools leave at once, so no new call starts against an island that
    // is about to reload.
    expect(context.live()).toEqual([]);
    await flush();
    // The call has its error result, and the recovery is still held.
    expect(pending.settled).toBe(true);
    expect(pending.value).toBe(refused);
    order.push("result");
    expect(order).toEqual(["result"]);
    // The hold gives the browser time to hand the result to the agent.
    expect(scheduler.timeouts.map((timeout) => timeout.delay)).toEqual([AGENT_RESULT_HOLD_MS]);
    scheduler.runAll();
    await released;
    expect(order).toEqual(["result", "recovery"]);
  });

  it("agent_012 every pending call of the island has its result before the recovery", async () => {
    const { context, scheduler, host, island } = hostWithIsland();
    const first = settledState(context.call("counter.increment.main", { n: 1 }));
    const second = settledState(context.call("counter.increment.main", { n: 2 }));
    await flush();
    let answered = 0;
    const released = host.release(island, () => {
      answered += 1;
      return agentToolError(`answer ${String(answered)}`);
    });
    await flush();
    expect(first.settled && second.settled).toBe(true);
    expect([first.value?.isError, second.value?.isError]).toEqual([true, true]);
    scheduler.runAll();
    await released;
  });

  it("agent_012 a call made during a refusal is refused, not queued", async () => {
    const { context, scheduler, host, island, scheduled } = hostWithIsland();
    void context.call("counter.increment.main", { n: 1 });
    await flush();
    expect(scheduled).toEqual([{ n: 1 }]);
    const released = host.release(island, () => agentToolError("refused"));
    // WebMCP may still call the execute it was handed before the tools
    // left: the call is answered with an error and never reaches the island.
    const late = await context.call("counter.increment.main", { n: 2 });
    expect(late.isError).toBe(true);
    expect(scheduled).toEqual([{ n: 1 }]);
    scheduler.runAll();
    await released;
  });

  it("agent_012 a refused click with no pending call recovers at once", async () => {
    const { context, scheduler, host, island } = hostWithIsland();
    await host.release(island, () => agentToolError("unused"));
    expect(scheduler.timeouts).toEqual([]);
    // The tools are gone until the island registers them again.
    expect(context.live()).toEqual([]);
    host.restore(island);
    expect(context.live()).toEqual(["counter.increment.main"]);
  });

  it("agent_012 execute refuses once its island has left the page", async () => {
    const { context, host, island, scheduled } = hostWithIsland();
    const execute = context.registered[0]?.tool.execute;
    host.disconnect(island);
    expect(context.live()).toEqual([]);
    const result = await execute?.({});
    expect(result?.isError).toBe(true);
    expect(scheduled).toEqual([]);
  });

  it("agent_012 tools leave on suspend and register again on resume", async () => {
    const { context, host, scheduled } = hostWithIsland();
    const before = context.registered[0]?.tool.execute;
    host.suspend();
    expect(context.live()).toEqual([]);
    // A call through the suspended registration never reaches the island.
    expect((await before?.({}))?.isError).toBe(true);
    expect(scheduled).toEqual([]);
    host.resume();
    expect(context.live()).toEqual(["counter.increment.main"]);
    expect(context.registered).toHaveLength(2);
    void context.call("counter.increment.main", { n: 3 });
    expect(scheduled).toEqual([{ n: 3 }]);
  });

  it("agent_012 an island that cannot register now registers on resume", () => {
    const context = new FakeModelContext();
    const host = new AgentToolHost<object>(new ManualScheduler());
    const island = {};
    let ready = false;
    host.connect(island, () =>
      ready
        ? {
            context,
            tools: [TOOL],
            execute: () => Promise.resolve(agentToolError("unused")),
            onFailure: () => undefined,
          }
        : null,
    );
    expect(context.live()).toEqual([]);
    host.suspend();
    ready = true;
    host.resume();
    expect(context.live()).toEqual(["counter.increment.main"]);
  });
});
