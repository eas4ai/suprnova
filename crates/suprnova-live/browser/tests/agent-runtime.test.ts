/**
 * The page's WebMCP tool calls driven through the whole `DocumentRuntime`
 * (AGENT-012): islands discovered from a document, calls registered with a
 * model context, requests sent through the runtime's transport and their
 * responses applied, on a small DOM and with every port faked.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AGENT_RESULT_HOLD_MS } from "../src/agents/host.js";
import type {
  ModelContextLike,
  ModelContextTool,
  ModelContextToolResult,
} from "../src/agents/model-context.js";
import { canonicalize, type JsonValue } from "../src/canonical.js";
import { RuntimeCallRegistry } from "../src/extensions/calls.js";
import { EffectRegistry } from "../src/extensions/effects.js";
import { DocumentRuntime } from "../src/islands/discovery.js";
import { IdiomorphAdapter } from "../src/morph/idiomorph.js";
import { parseRuntimeConfig } from "../src/runtime/config.js";
import type { RuntimeDiagnosticInput, RuntimeDiagnosticSink } from "../src/runtime/diagnostics.js";
import type { RuntimePorts } from "../src/runtime/ports.js";
import type { RuntimeConfig } from "../src/runtime/types.js";
import {
  RuntimeDocument,
  RuntimeElement,
  RuntimeMutationObserver,
  stubDomGlobals,
} from "./support/runtime-dom.js";

const COMPONENT = "tests.counter";

const CONFIG = {
  asset_identity: "runtime-test-v1",
  async_max_queued_events: 4_096,
  async_max_replay_events: 4_096,
  credentials: "same-origin",
  endpoint: "/_suprnova/live",
  max_html_bytes: 1_048_576,
  max_json_depth: 32,
  max_json_entries: 1_000_000,
  max_parallel_per_island: 1,
  max_queued_per_island: 16,
  max_redirect_bytes: 65_536,
  max_request_bytes: 16_777_216,
  max_request_items: 65_536,
  max_response_bytes: 1_048_576,
  max_response_items: 65_536,
  morph_deadline_ms: 0,
  morph_max_attributes: 10_000_000,
  morph_max_attributes_per_element: 4_096,
  morph_max_depth: 512,
  morph_max_keys: 1_000_000,
  morph_max_nodes: 1_000_000,
  protocol: { maximum: 2, minimum: 1 },
  request_timeout_ms: 15_000,
  runtime_contract_version: 1,
  upload_chunk_bytes: 8_388_608,
  upload_max_active: 8,
  upload_max_file_bytes: 1_073_741_824,
  upload_max_pending_bytes: 4_294_967_296,
  upload_max_pending_files: 1_024,
};

function runtimeConfig(): RuntimeConfig {
  const element = {
    textContent: JSON.stringify(CONFIG),
    getAttribute: (name: string) => (name === "type" ? "application/json" : null),
  };
  return parseRuntimeConfig({
    baseURI: "https://app.example.test/page",
    querySelectorAll: () => [element],
  } as unknown as Document);
}

function base64Url(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}

function identity(seed: number, length: number): string {
  return base64Url(Uint8Array.from({ length }, (_, index) => (seed + index) % 256));
}

/** A signed-looking instance snapshot envelope of the counter. */
function envelope(slot: string, instance: string, revision: bigint, state: JsonValue): JsonValue {
  return {
    body: {
      component: { name: COMPONENT },
      form: "instance",
      instance_id: instance,
      revision: revision.toString(10),
      slot,
      state,
    },
    signature: identity(revision === 1n ? 200 : 210, 32),
  };
}

function encoded(value: JsonValue): string {
  return Buffer.from(canonicalize(value), "utf8").toString("base64url");
}

const TAG = (value: string): JsonValue => ({ $live: "u64", value });

/** One counter island keyed `key`, offering `counter.add.{key}`. */
function island(document: RuntimeDocument, key: string, seed: number): RuntimeElement {
  const slot = `counter-${key}`;
  const instance = identity(seed, 16);
  const root = new RuntimeElement(document, "div", {
    "data-suprnova-live-component": COMPONENT,
    "data-suprnova-live-contract": "1",
    "data-suprnova-live-document-key": key,
    "data-suprnova-live-instance": instance,
    "data-suprnova-live-island": "",
    "data-suprnova-live-lazy-complete": "true",
    "data-suprnova-live-protocol-min": "2",
    "data-suprnova-live-revision": "1",
    "data-suprnova-live-root": slot,
    "data-suprnova-live-slot": slot,
    "data-suprnova-live-snapshot": encoded(
      envelope(slot, instance, 1n, { count: TAG("1"), shape: TAG("9") }),
    ),
    "data-suprnova-live-snapshot-kind": "instance",
    "data-suprnova-live-tools": JSON.stringify({
      v: 1,
      fields: ["count", "shape"],
      integers: ["count"],
      tools: [
        {
          action: "add",
          name: `counter.add.${key}`,
          description: "Add one to the counter.",
          inputSchema: { type: "object", properties: {}, additionalProperties: false },
          annotations: {
            readOnlyHint: false,
            untrustedContentHint: false,
            consequentialHint: true,
          },
        },
      ],
    }),
  });
  root.append(
    new RuntimeElement(document, "button", { "live:click": "touch", id: `touch-${key}` }),
  );
  return root;
}

/** A model context that records each registration and its signal. */
class FakeModelContext implements ModelContextLike {
  readonly registered: { tool: ModelContextTool; signal: AbortSignal | undefined }[] = [];

  registerTool(
    tool: ModelContextTool,
    options?: Readonly<{ signal?: AbortSignal }>,
  ): Promise<void> {
    this.registered.push({ tool, signal: options?.signal });
    return Promise.resolve();
  }

  live(): string[] {
    return this.registered
      .filter((entry) => entry.signal?.aborted !== true)
      .map((entry) => entry.tool.name)
      .sort();
  }

  call(name: string, input: unknown = {}): Promise<ModelContextToolResult> {
    const entries = this.registered.filter(
      (entry) => entry.tool.name === name && entry.signal?.aborted !== true,
    );
    const entry = entries[entries.length - 1];
    if (entry === undefined) throw new Error(`no live registration of ${name}`);
    return entry.tool.execute(input);
  }
}

interface SentRequest {
  readonly body: Record<string, unknown>;
  readonly tool: string | null;
  readonly mediaType: string;
  respond(status: number, response: Record<string, unknown>): void;
}

/** A transport whose responses the test sends. */
class FakeTransport {
  readonly sent: SentRequest[] = [];

  fetch = (_input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const headers = new Headers(init?.headers);
    const text = typeof init?.body === "string" ? init.body : "{}";
    const body = JSON.parse(text) as Record<string, unknown>;
    return new Promise<Response>((resolve) => {
      const mediaType = headers.get("content-type") ?? "";
      this.sent.push({
        body,
        tool: headers.get("suprnova-tool-call"),
        mediaType,
        respond(status, response) {
          resolve(
            new Response(
              canonicalize({ ...response, correlation_id: body["correlation_id"] } as JsonValue),
              { status, headers: { "content-type": mediaType } },
            ),
          );
        },
      });
    });
  };

  /** The request of the action `action` on the island keyed `key`, once sent. */
  async request(key: string, action: string): Promise<SentRequest> {
    const find = (): SentRequest | undefined =>
      this.sent.find((request) => {
        const extensions = request.body["extensions"] as Record<string, unknown>;
        const operations = request.body["operations"] as { name?: string }[];
        return (
          extensions["x_suprnova_live_document_key_v1"] === key &&
          operations.some((operation) => operation.name === action)
        );
      });
    await until(() => find() !== undefined, `the request of ${action} on ${key}`);
    const found = find();
    if (found === undefined) throw new Error(`no request of ${action} on ${key}`);
    return found;
  }
}

/** A scheduler whose timeouts run only when the test says, and the delays asked. */
class ManualScheduler {
  readonly timeouts = new Map<number, { callback: VoidFunction; delay: number }>();
  #next = 1;

  microtask(callback: VoidFunction): void {
    queueMicrotask(callback);
  }

  animationFrame(callback: FrameRequestCallback): number {
    return this.timeout(() => {
      callback(0);
    }, 16);
  }

  cancelAnimationFrame(handle: number): void {
    this.timeouts.delete(handle);
  }

  timeout(callback: VoidFunction, delay: number): number {
    const handle = this.#next++;
    this.timeouts.set(handle, { callback, delay });
    return handle;
  }

  clearTimeout(handle: number): void {
    this.timeouts.delete(handle);
  }

  /** Run every timeout of `delay` milliseconds now. */
  run(delay: number): void {
    for (const [handle, timeout] of [...this.timeouts]) {
      if (timeout.delay !== delay) continue;
      this.timeouts.delete(handle);
      timeout.callback();
    }
  }

  delays(): number[] {
    return [...this.timeouts.values()].map((timeout) => timeout.delay);
  }
}

/**
 * Wait until `condition` holds: the runtime reads responses through real
 * streams and promises, so the turns it takes vary.
 */
async function until(condition: () => boolean, what: string): Promise<void> {
  for (let turn = 0; turn < 400; turn += 1) {
    if (condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error(`timed out waiting for ${what}`);
}

interface Page {
  readonly document: RuntimeDocument;
  readonly context: FakeModelContext;
  readonly transport: FakeTransport;
  readonly scheduler: ManualScheduler;
  readonly navigation: { reloads: number; order: string[] };
  readonly runtime: DocumentRuntime;
  readonly diagnostics: RuntimeDiagnosticInput[];
}

function page(keys: readonly string[] = ["main", "side"]): Page {
  const document = new RuntimeDocument();
  const context = new FakeModelContext();
  document.modelContext = context;
  keys.forEach((key, index) => {
    document.body.append(island(document, key, 16 * (index + 1)));
  });
  const transport = new FakeTransport();
  const scheduler = new ManualScheduler();
  const navigation = { reloads: 0, order: [] as string[] };
  const ports: RuntimePorts = {
    clock: { now: () => 1_000 },
    connectivity: { isOnline: () => true },
    randomness: {
      randomBytes: (length) => Uint8Array.from({ length }, (_, index) => (index * 7 + 3) % 256),
    },
    transport: { fetch: transport.fetch },
    navigation: {
      assign: () => {
        navigation.order.push("assign");
      },
      replace: () => undefined,
      reload: () => {
        navigation.reloads += 1;
        navigation.order.push("reload");
      },
    },
    observers: {
      mutation: (callback) => new RuntimeMutationObserver(document, callback),
      intersection: () => null,
    },
    scheduler,
    features: {
      prefersReducedMotion: () => true,
      supportsViewTransitions: () => false,
      supportsSpeculationRules: () => false,
    },
  };
  const diagnostics: RuntimeDiagnosticInput[] = [];
  const sink: RuntimeDiagnosticSink = {
    record: (input) => {
      diagnostics.push(input);
    },
  };
  const config = runtimeConfig();
  const runtime = new DocumentRuntime(
    document as unknown as Document,
    config,
    sink,
    ports,
    new EffectRegistry({ diagnostics: sink, scheduler }),
    new RuntimeCallRegistry({ diagnostics: sink, scheduler }),
    new IdiomorphAdapter(),
  );
  runtime.start();
  return { document, context, transport, scheduler, navigation, runtime, diagnostics };
}

beforeEach(() => {
  stubDomGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function settledState<T>(promise: Promise<T>): { settled: boolean; value?: T } {
  const state: { settled: boolean; value?: T } = { settled: false };
  void promise.then((value) => {
    state.settled = true;
    state.value = value;
  });
  return state;
}

/** The island root keyed `key`. */
function root(document: RuntimeDocument, key: string): RuntimeElement {
  const element = document.querySelector(`[data-suprnova-live-document-key="${key}"]`);
  if (element === null) throw new Error(`no island ${key}`);
  return element;
}

/** A trusted click on the `touch` button of the island keyed `key`. */
function click(document: RuntimeDocument, key: string): void {
  const island = root(document, key);
  const button = island.querySelector("button");
  if (button === null) throw new Error(`no button in ${key}`);
  const path = [button, island, document.body, document.documentElement, document];
  document.dispatch("click", {
    type: "click",
    target: button,
    currentTarget: document,
    composedPath: () => path,
    isTrusted: true,
    defaultPrevented: false,
    cancelBubble: false,
    button: 0,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    preventDefault() {
      Object.defineProperty(this, "defaultPrevented", { value: true });
    },
    stopPropagation() {
      Object.defineProperty(this, "cancelBubble", { value: true });
    },
  });
}

/** The protocol 2 answer that the island must refresh. */
function refreshRequired(): Record<string, unknown> {
  return {
    child_deliveries: [],
    effects: [],
    error: { category: "authorization", detail: "denied", recovery: "refresh_island" },
    events: [],
    extensions: {},
    outcome: "refresh_required",
    protocol_version: 2,
    url_intent: null,
    validation: {},
  };
}

/** The protocol 2 answer that commits revision 2 of `key` with `state`, rendering nothing. */
function committed(key: string, seed: number, state: JsonValue): Record<string, unknown> {
  return {
    accepted_revision: "2",
    child_deliveries: [],
    effects: [],
    events: [],
    extensions: {},
    outcome: "accepted",
    protocol_version: 2,
    render: { kind: "no_render" },
    snapshot: envelope(`counter-${key}`, identity(seed, 16), 2n, state),
    url_intent: null,
    validation: {},
  };
}

describe("the page's WebMCP tool calls through the runtime", () => {
  it("agent_012 registers each island's tools when the runtime starts", () => {
    const { context, diagnostics } = page();
    expect(diagnostics).toEqual([]);
    expect(context.live()).toEqual(["counter.add.main", "counter.add.side"]);
  });

  it("agent_012 a reload for another island answers this island's pending call first", async () => {
    const { document, context, transport, scheduler, navigation } = page();
    const call = settledState(context.call("counter.add.main"));
    // `main`'s tool call is in flight, its response held back.
    expect((await transport.request("main", "add")).tool).toBe("counter.add.main");
    click(document, "side");
    (await transport.request("side", "touch")).respond(409, refreshRequired());
    // The page reloads to recover `side`: first `main`'s call has its
    // result, and every island's tools are gone, so none starts meanwhile.
    await until(() => call.settled, "main's call to settle");
    expect(navigation.reloads).toBe(0);
    expect(call.value?.isError).toBe(true);
    expect(context.live()).toEqual([]);
    expect(scheduler.delays()).toContain(AGENT_RESULT_HOLD_MS);
    scheduler.run(AGENT_RESULT_HOLD_MS);
    await until(() => navigation.reloads === 1, "the reload");
  });

  it("agent_012 a refused call's recovery waits for the call's result", async () => {
    const { context, transport, scheduler, navigation } = page();
    const call = settledState(context.call("counter.add.main"));
    (await transport.request("main", "add")).respond(409, refreshRequired());
    await until(() => call.settled, "the call to settle");
    expect(call.value?.isError).toBe(true);
    expect(navigation.reloads).toBe(0);
    scheduler.run(AGENT_RESULT_HOLD_MS);
    await until(() => navigation.reloads === 1, "the reload");
    expect(navigation.order).toEqual(["reload"]);
  });

  it("agent_012 a refused click with no call pending recovers at once", async () => {
    const { document, transport, scheduler, navigation } = page();
    click(document, "side");
    (await transport.request("side", "touch")).respond(409, refreshRequired());
    await until(() => navigation.reloads === 1, "the reload");
    expect(scheduler.delays()).not.toContain(AGENT_RESULT_HOLD_MS);
  });

  it("agent_012 tools leave on suspend and register again on resume", () => {
    const { context, runtime } = page();
    runtime.suspend();
    expect(context.live()).toEqual([]);
    runtime.resume();
    expect(context.live()).toEqual(["counter.add.main", "counter.add.side"]);
    expect(context.registered).toHaveLength(4);
  });

  it("agent_012 a call refuses once its island has left the page", async () => {
    const { document, context, transport } = page();
    const registration = context.registered.find((entry) => entry.tool.name === "counter.add.main");
    root(document, "main").remove();
    await until(() => context.live().length === 1, "main's tools to leave");
    expect(context.live()).toEqual(["counter.add.side"]);
    const result = await registration?.tool.execute({});
    expect(result?.isError).toBe(true);
    expect(transport.sent).toEqual([]);
  });

  it("agent_012 a call's result unwraps only the fields the root marks as integers", async () => {
    const { context, transport } = page();
    const call = context.call("counter.add.main");
    (await transport.request("main", "add")).respond(
      200,
      committed("main", 16, { count: TAG("2"), shape: TAG("9") }),
    );
    const result = await call;
    expect(result.isError).toBe(false);
    // `count` is a u64 field; `shape` holds the same shape in a Json field
    // and is shown as the snapshot carries it.
    expect(result.structuredContent).toEqual({
      outcome: "no_render",
      fields: { count: "2", shape: { $live: "u64", value: "9" } },
    });
  });
});
