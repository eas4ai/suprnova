import { describe, expect, it } from "vitest";

import {
  agentFields,
  agentToolError,
  agentToolResult,
  argumentNames,
} from "../src/agents/result.js";
import {
  modelContextOf,
  registerAgentTools,
  type ModelContextLike,
  type ModelContextTool,
} from "../src/agents/model-context.js";
import type { ValidatedResponse } from "../src/application/types.js";
import { EventRouter } from "../src/directives/events.js";
import { DirectiveOwnership } from "../src/directives/ownership.js";
import {
  IslandMetadataError,
  parseAgentTools,
  type IslandMetadata,
} from "../src/islands/metadata.js";
import type { IslandRecord } from "../src/islands/record.js";
import type { RuntimeDiagnosticInput } from "../src/runtime/diagnostics.js";
import type { DelegatedListenerRegistry } from "../src/runtime/listeners.js";
import type { RuntimeScheduler } from "../src/runtime/ports.js";
import { ServerIntent } from "../src/scheduler/intent.js";
import type { SchedulerTicket } from "../src/scheduler/types.js";
import { fetchLiveRequest, TOOL_CALL_HEADER } from "../src/transport/fetch.js";
import { LiveRequestBuilder } from "../src/transport/request.js";
import { SERVER_DEFAULT_LIMITS } from "../src/limits.js";

const TOOL = {
  action: "increment",
  name: "counter.increment.main",
  title: "Increment",
  description: "Add one to the counter.",
  inputSchema: { type: "object", properties: {}, additionalProperties: false },
  annotations: { readOnlyHint: false, untrustedContentHint: false, consequentialHint: true },
};

function contract(
  tools: readonly unknown[] = [TOOL],
  fields: readonly string[] = ["count"],
  integers: readonly string[] = ["count"],
) {
  return JSON.stringify({ v: 1, fields, integers, tools });
}

function defined<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("expected a value");
  return value;
}

function invalid(value: string, key = "main"): void {
  expect(() => parseAgentTools(value, key)).toThrow(IslandMetadataError);
}

describe("the island root's tool contract", () => {
  it("agent_012 parses tools named for the island's key", () => {
    const parsed = parseAgentTools(contract(), "main");
    expect(parsed.fields).toEqual(["count"]);
    expect(parsed.integers).toEqual(["count"]);
    expect(parsed.tools).toHaveLength(1);
    expect(parsed.tools[0]).toMatchObject({
      action: "increment",
      name: "counter.increment.main",
      title: "Increment",
      description: "Add one to the counter.",
      annotations: { readOnlyHint: false, untrustedContentHint: false, consequentialHint: true },
    });
    expect(Object.isFrozen(parsed.tools[0])).toBe(true);
  });

  it("agent_012 refuses a contract the server did not build for this island", () => {
    // A name that does not end in this island's key.
    invalid(contract([{ ...TOOL, name: "counter.increment.other" }]));
    // A key outside the tool instance grammar.
    invalid(contract([{ ...TOOL, name: "counter.increment.main.left" }]), "main.left");
    invalid(contract(), "a:b");
    // A name that is only the key.
    invalid(contract([{ ...TOOL, name: ".main" }]));
    // Shapes and hints outside the contract.
    invalid(contract([{ ...TOOL, description: "  " }]));
    invalid(contract([{ ...TOOL, inputSchema: [] }]));
    invalid(contract([{ ...TOOL, annotations: { readOnlyHint: true } }]));
    invalid(contract([{ ...TOOL, annotations: { ...TOOL.annotations, destructiveHint: true } }]));
    invalid(contract([{ ...TOOL, extra: true }]));
    invalid(contract([TOOL, TOOL]));
    invalid(contract([]));
    invalid(
      contract(
        Array.from({ length: 65 }, (_, index) => ({ ...TOOL, name: `t${String(index)}.main` })),
      ),
    );
    invalid(contract([TOOL], ["has space"]));
    // The 64-bit integer fields are agent-visible fields, each once.
    invalid(contract([TOOL], ["count"], ["other"]));
    invalid(contract([TOOL], ["count"], ["count", "count"]));
    invalid(JSON.stringify({ v: 1, fields: ["count"], tools: [TOOL] }));
    invalid(JSON.stringify({ v: 2, fields: [], integers: [], tools: [TOOL] }));
    invalid("not json");
  });
});

class FakeModelContext implements ModelContextLike {
  readonly registered: { tool: ModelContextTool; signal: AbortSignal | undefined }[] = [];
  refuse = false;

  registerTool(
    tool: ModelContextTool,
    options?: Readonly<{ signal?: AbortSignal }>,
  ): Promise<void> {
    this.registered.push({ tool, signal: options?.signal });
    return this.refuse
      ? Promise.reject(new DOMException("duplicate", "InvalidStateError"))
      : Promise.resolve();
  }
}

const DEFINITION = defined(parseAgentTools(contract(), "main").tools[0]);

describe("WebMCP registration", () => {
  it("agent_012 finds the model context only where the browser exposes registerTool", () => {
    const context = new FakeModelContext();
    expect(modelContextOf({ modelContext: context } as unknown as Document)).toBe(context);
    expect(
      modelContextOf({
        defaultView: { navigator: { modelContext: context } },
      } as unknown as Document),
    ).toBe(context);
    expect(modelContextOf({ modelContext: {} } as unknown as Document)).toBeNull();
    expect(modelContextOf({} as unknown as Document)).toBeNull();
  });

  it("agent_012 registers each tool with its own signal and aborts them together", async () => {
    const context = new FakeModelContext();
    const calls: unknown[] = [];
    const abort = registerAgentTools(
      context,
      [DEFINITION],
      (tool, input) => {
        calls.push([tool.action, input]);
        return Promise.resolve(agentToolError("unused"));
      },
      () => {
        throw new Error("no refusal expected");
      },
    );
    expect(context.registered).toHaveLength(1);
    const registered = context.registered[0];
    expect(registered?.tool).toMatchObject({
      name: "counter.increment.main",
      title: "Increment",
      description: "Add one to the counter.",
      inputSchema: TOOL.inputSchema,
      annotations: TOOL.annotations,
    });
    await registered?.tool.execute({ amount: 1 });
    expect(calls).toEqual([["increment", { amount: 1 }]]);
    expect(registered?.signal?.aborted).toBe(false);
    abort();
    expect(registered?.signal?.aborted).toBe(true);
    abort();
  });

  it("agent_012 reports a registration the browser refused", async () => {
    const context = new FakeModelContext();
    context.refuse = true;
    const failures: string[] = [];
    registerAgentTools(
      context,
      [DEFINITION],
      () => Promise.resolve(agentToolError("unused")),
      (failure) => failures.push(failure),
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(failures).toEqual(["rejected"]);
  });
});

function committed(
  state: Readonly<Record<string, unknown>>,
  validation: Readonly<Record<string, unknown>> = {},
): ValidatedResponse {
  return {
    kind: "committed",
    outcome: "accepted",
    protocol: 2,
    correlationId: "x",
    effects: [],
    events: [],
    extensions: {},
    validation,
    acceptedRevision: 1n,
    childDeliveries: [],
    render: { kind: "html", html: "<p>secret html</p>" },
    snapshot: { body: { state }, signature: "s" },
    snapshotView: {} as never,
    reflectedUrl: null,
  } as unknown as ValidatedResponse;
}

describe("the tool result", () => {
  it("agent_012 tool result carries only the agent-visible fields", () => {
    const result = agentToolResult(
      committed({
        count: { $live: "u64", value: "18446744073709551615" },
        label: "Main",
        owner: "alice",
      }),
      { fields: ["count", "label"], integers: ["count"], arguments: [] },
    );
    expect(result.isError).toBe(false);
    expect(result.structuredContent).toEqual({
      outcome: "accepted",
      fields: { count: "18446744073709551615", label: "Main" },
    });
    expect(result.content[0]?.text).not.toContain("alice");
    expect(result.content[0]?.text).not.toContain("secret html");
    expect(agentFields({ body: { state: { owner: "x" } } }, [], [])).toEqual({});
  });

  it("agent_012 only a field the server marks as a 64-bit integer is unwrapped", () => {
    const tag = { $live: "u64", value: "5" };
    const result = agentToolResult(committed({ count: tag, shape: tag }), {
      fields: ["count", "shape"],
      integers: ["count"],
      arguments: [],
    });
    // `shape` has the Json codec: it is shown as the snapshot carries it,
    // as the server's headless result shows it.
    expect(result.structuredContent).toEqual({
      outcome: "accepted",
      fields: { count: "5", shape: { $live: "u64", value: "5" } },
    });
  });

  it("agent_012 a validation outcome is an error naming the issues", () => {
    const result = agentToolResult(committed({ count: 3 }, { count: "validation.range" }), {
      fields: ["count"],
      integers: [],
      arguments: [],
    });
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toMatchObject({
      outcome: "invalid",
      validation: { count: ["validation.range"] },
    });
  });

  it("agent_013 validation names only visible fields and the tool's arguments", () => {
    const result = agentToolResult(
      committed(
        { count: 3 },
        {
          owner: { id: "validation.length", text: "The owner is too long." },
          title: { id: "validation.length", text: "The title is too long." },
          "count.inner": "validation.range",
        },
      ),
      { fields: ["count"], integers: [], arguments: ["title"] },
    );
    expect(result.isError).toBe(true);
    // The owner is neither agent-visible nor an argument: one generic
    // issue, with no field name and no message.
    expect(result.structuredContent).toEqual({
      outcome: "invalid",
      fields: { count: 3 },
      validation: {
        "": [],
        "count.inner": ["validation.range"],
        title: ["The title is too long."],
      },
    });
    expect(result.content[0]?.text).not.toContain("owner");
    // The arguments are the properties the tool's input schema lists.
    expect(
      argumentNames({
        ...TOOL,
        inputSchema: { type: "object", properties: { title: { type: "string" } } },
      }),
    ).toEqual(["title"]);
    expect(argumentNames({ ...TOOL, inputSchema: { type: "object" } })).toEqual([]);
  });

  it("agent_012 a refused action is an error", () => {
    const result = agentToolResult(
      {
        kind: "recovery",
        outcome: "refresh_required",
        recovery: "refresh_island",
        error: { category: "snapshot", detail: "x", recovery: "refresh_island" },
      } as unknown as ValidatedResponse,
      { fields: ["count"], integers: [], arguments: [] },
    );
    expect(result.isError).toBe(true);
    expect(result.structuredContent).not.toHaveProperty("fields");
  });
});

const SCHEDULER: RuntimeScheduler = {
  animationFrame: () => 1,
  cancelAnimationFrame: () => undefined,
  clearTimeout: () => undefined,
  microtask: (callback) => {
    callback();
  },
  timeout: () => 1,
};

function island(form: "seed" | "instance") {
  const root = {
    nodeType: 1,
    attributes: [],
    children: Object.assign([], { item: () => null }),
    isConnected: true,
    parentElement: null,
    shadowRoot: null,
    matches: (selector: string) => selector === "[data-suprnova-live-island]",
    hasAttribute: () => false,
    getAttribute: () => null,
    setAttribute: () => undefined,
  } as unknown as Element;
  const metadata = {
    component: "fixture.counter",
    documentKey: "main",
    instanceId: form === "instance" ? "MDEyMzQ1Njc4OTo7PD0-Pw" : null,
    lazyComplete: false,
    protocolMinimum: 2,
    revision: form === "instance" ? 1n : 0n,
    runtimeContract: 1,
    slot: "counter",
    snapshot: Object.freeze({}),
    snapshotForm: form,
  } as IslandMetadata;
  const intents: ServerIntent[] = [];
  const record = {
    element: root,
    metadata,
    active: () => true,
    enqueueTicket: (intent: ServerIntent) => {
      intents.push(intent);
      return { intent, sequence: intents.length } as unknown as SchedulerTicket;
    },
    onDispose: () => undefined,
  } as unknown as IslandRecord;
  const diagnostics: RuntimeDiagnosticInput[] = [];
  const ownership = new DirectiveOwnership();
  const router = new EventRouter(
    { add: () => () => undefined } as unknown as DelegatedListenerRegistry,
    ownership,
    { randomBytes: (length) => new Uint8Array(length).fill(7) },
    { now: () => 0 },
    SCHEDULER,
    {
      record(diagnostic: RuntimeDiagnosticInput) {
        diagnostics.push(diagnostic);
      },
    },
  );
  router.connect(record, ownership.connect(record));
  return { router, record, intents, diagnostics };
}

describe("scheduling a tool call", () => {
  it("agent_012 schedules the action as a click does and names the tool", async () => {
    const { router, record, intents } = island("instance");
    const scheduled = router.scheduleAgentAction(record, "increment", "counter.increment.main", {
      amount: 2,
    });
    expect(scheduled).not.toBeNull();
    expect(intents).toHaveLength(1);
    const intent = defined(intents[0]);
    expect(intent.source.agentTool).toBe("counter.increment.main");
    expect(intent.source.eventType).toBe("agent_tool");
    expect(intent.operations).toEqual([
      { kind: "invoke_action", name: "increment", arguments: { amount: 2 } },
    ]);
    expect(intent.promotionNonce()).toBeNull();

    // The request it becomes carries the tool-call header, beside the
    // island's snapshot and the Live media type.
    const request = await new LiveRequestBuilder().build({
      intent,
      protocolVersion: 2,
      randomness: { randomBytes: (length) => new Uint8Array(length).fill(3) },
    });
    expect(request.toolCall).toBe("counter.increment.main");
    const seen: Headers[] = [];
    await fetchLiveRequest(request, {
      endpoint: new URL("https://app.example/__live/action"),
      credentials: "same-origin",
      requestTimeoutMs: 1_000,
      limits: SERVER_DEFAULT_LIMITS,
      scheduler: SCHEDULER,
      isOnline: () => true,
      transport: {
        fetch: (_url: URL | string, init?: RequestInit) => {
          seen.push(new Headers(init?.headers));
          return Promise.reject(new Error("offline"));
        },
      },
    }).catch(() => undefined);
    expect(seen[0]?.get(TOOL_CALL_HEADER)).toBe("counter.increment.main");
    expect(seen[0]?.get("content-type")).toContain("application/vnd.suprnova.live+json");
  });

  it("agent_012 a call on a public seed promotes it, as a click does", () => {
    const { router, record, intents } = island("seed");
    router.scheduleAgentAction(record, "increment", "counter.increment.main", {});
    expect(intents[0]?.promotionNonce()).not.toBeNull();
  });

  it("agent_012 an ordinary action request carries no tool-call header", async () => {
    const { record } = island("instance");
    const click = new ServerIntent(
      Object.freeze({
        directive: null,
        element: record.element,
        eventType: "click",
        island: record,
        trusted: true,
      }),
      [{ kind: "invoke_action", name: "increment", arguments: {} }],
      null,
    );
    const request = await new LiveRequestBuilder().build({
      intent: click,
      protocolVersion: 2,
      randomness: { randomBytes: (length) => new Uint8Array(length).fill(4) },
    });
    expect(request.toolCall).toBeUndefined();
  });
});
