/**
 * The page's server tools in the Live runtime (AGENT-014): the element the
 * server writes, the call a tool makes, and the result it maps, checked
 * against the shared cases every browser implementation reads
 * (`framework/tests/agents/fixtures/browser-calls.json`), and the
 * registration, the call and the refresh after a write tool driven through
 * the whole `DocumentRuntime`.
 */
import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  ModelContextLike,
  ModelContextTool,
  ModelContextToolResult,
} from "../src/agents/model-context.js";
import {
  SERVER_TOOLS_ELEMENT_ID,
  ServerToolsError,
  buildServerCall,
  callServerTool,
  csrfHeader,
  failureResult,
  parseServerTools,
  resultOfResponse,
  type ServerToolDefinition,
  type ServerToolPathPiece,
} from "../src/agents/server-tools.js";
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
  RuntimeNode,
  stubDomGlobals,
} from "./support/runtime-dom.js";

interface PathCase {
  readonly name: string;
  readonly method: string;
  readonly path: ServerToolPathPiece[];
  readonly arguments: Record<string, unknown>;
  readonly expected:
    { readonly url: string; readonly body: unknown } | { readonly error: ModelContextToolResult };
}

interface CsrfCase {
  readonly name: string;
  readonly meta: string | null;
  readonly cookie: string | null;
  readonly expected: Record<string, string>;
}

interface ResultCase {
  readonly name: string;
  readonly maxResultBytes: number;
  readonly response:
    | {
        readonly status: number;
        readonly type?: string;
        readonly contentType: string | null;
        readonly body: string;
      }
    | { readonly failure: "network" }
    | { readonly failure: "timeout"; readonly callTimeoutMs: number };
  readonly expected: ModelContextToolResult;
}

interface Fixture {
  readonly paths: PathCase[];
  readonly csrf: CsrfCase[];
  readonly results: ResultCase[];
}

const FIXTURE = JSON.parse(
  readFileSync(
    new URL("../../../../framework/tests/agents/fixtures/browser-calls.json", import.meta.url),
    "utf8",
  ),
) as Fixture;

describe("the shared browser call cases", () => {
  it("fills every path and query as the cases say", () => {
    expect(FIXTURE.paths.length).toBeGreaterThan(0);
    for (const testCase of FIXTURE.paths) {
      const call = buildServerCall(testCase.method, testCase.path, testCase.arguments);
      if ("error" in testCase.expected) {
        expect(call, testCase.name).toEqual({ kind: "refused", result: testCase.expected.error });
        continue;
      }
      expect(call.kind, testCase.name).toBe("send");
      if (call.kind !== "send") continue;
      expect(call.url, testCase.name).toBe(testCase.expected.url);
      if (testCase.expected.body === null) {
        expect(call.body, testCase.name).toBeNull();
      } else {
        expect(JSON.parse(call.body ?? "null"), testCase.name).toEqual(testCase.expected.body);
      }
    }
  });

  it("chooses the CSRF header as the cases say", () => {
    expect(FIXTURE.csrf.length).toBeGreaterThan(0);
    for (const testCase of FIXTURE.csrf) {
      expect(csrfHeader(testCase.meta, testCase.cookie ?? ""), testCase.name).toEqual(
        testCase.expected,
      );
    }
  });

  it("maps every answer as the cases say", async () => {
    expect(FIXTURE.results.length).toBeGreaterThan(0);
    for (const testCase of FIXTURE.results) {
      const response = testCase.response;
      let result: ModelContextToolResult;
      if ("failure" in response) {
        result =
          response.failure === "network"
            ? failureResult({ kind: "network" })
            : failureResult({ kind: "timeout", callTimeoutMs: response.callTimeoutMs });
      } else if (response.type === "opaqueredirect") {
        result = await resultOfResponse(
          { type: "opaqueredirect", status: 0, headers: new Headers(), body: null },
          testCase.maxResultBytes,
        );
      } else {
        const headers: Record<string, string> =
          response.contentType === null ? {} : { "content-type": response.contentType };
        result = await resultOfResponse(
          new Response(response.body === "" ? null : response.body, {
            status: response.status,
            headers,
          }),
          testCase.maxResultBytes,
        );
      }
      expect(result, testCase.name).toEqual(testCase.expected);
    }
  });
});

const TOOL: ServerToolDefinition = Object.freeze({
  name: "posts.create",
  title: "Create a post",
  description: "Create a draft post for the signed-in user.",
  inputSchema: Object.freeze({ type: "object" }),
  annotations: Object.freeze({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
  }),
  method: "POST",
  path: Object.freeze([Object.freeze({ text: "/posts" })]),
});

const READ: ServerToolDefinition = Object.freeze({
  ...TOOL,
  name: "notes.show",
  annotations: Object.freeze({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
  }),
  method: "GET",
  path: Object.freeze([
    Object.freeze({ text: "/notes/" }),
    Object.freeze({ param: "note", optional: false, catchAll: false }),
  ]),
});

function payload(tools: unknown[] = [TOOL, READ]): string {
  return JSON.stringify({
    v: 1,
    limits: { maxResultBytes: 1024, callTimeoutMs: 30_000 },
    tools,
  });
}

describe("the server tools element", () => {
  it("reads a valid payload", () => {
    const parsed = parseServerTools(payload());
    expect(parsed.limits).toEqual({ maxResultBytes: 1024, callTimeoutMs: 30_000 });
    expect(parsed.tools.map((tool) => tool.name)).toEqual(["posts.create", "notes.show"]);
    expect(parsed.tools[1]?.path).toEqual(READ.path);
  });

  it.each([
    [
      "an extra key",
      { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [TOOL], x: 1 },
    ],
    ["another version", { v: 2, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [TOOL] }],
    ["no tool", { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [] }],
    ["a zero bound", { v: 1, limits: { maxResultBytes: 0, callTimeoutMs: 1 }, tools: [TOOL] }],
    [
      "a tool with an extra key",
      { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [{ ...TOOL, x: 1 }] },
    ],
    [
      "a name outside the grammar",
      { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [{ ...TOOL, name: "a b" }] },
    ],
    [
      "two tools of one name",
      { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [TOOL, TOOL] },
    ],
    [
      "a blank description",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: [{ ...TOOL, description: " " }],
      },
    ],
    [
      "a null title",
      { v: 1, limits: { maxResultBytes: 1, callTimeoutMs: 1 }, tools: [{ ...TOOL, title: null }] },
    ],
    [
      "a missing hint",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: [{ ...TOOL, annotations: { readOnlyHint: true, untrustedContentHint: false } }],
      },
    ],
    [
      "an unknown method",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: [{ ...TOOL, method: "TRACE" }],
      },
    ],
    [
      "a path that does not start with text",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: [{ ...TOOL, path: [{ param: "post", optional: false, catchAll: false }] }],
      },
    ],
    [
      "a path piece of another shape",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: [{ ...TOOL, path: [{ text: "/posts/" }, { param: "post" }] }],
      },
    ],
    [
      "more than 64 tools",
      {
        v: 1,
        limits: { maxResultBytes: 1, callTimeoutMs: 1 },
        tools: Array.from({ length: 65 }, (_, index) => ({
          ...TOOL,
          name: `tool.${String(index)}`,
        })),
      },
    ],
  ])("refuses %s", (_name, value) => {
    expect(() => parseServerTools(JSON.stringify(value))).toThrow(ServerToolsError);
  });

  it("refuses text that is not JSON", () => {
    expect(() => parseServerTools("{")).toThrow(ServerToolsError);
  });
});

interface Sent {
  readonly url: string;
  readonly init: RequestInit;
}

/** Options for one call, recording what it sends and what it refreshes. */
function callOptions(answer: (sent: Sent) => Promise<Response>) {
  const sent: Sent[] = [];
  const refreshes: number[] = [];
  const timers = new Map<number, VoidFunction>();
  let next = 1;
  return {
    sent,
    refreshes,
    timers,
    options: {
      origin: "https://app.example.test",
      fetch: (url: string, init: RequestInit): Promise<Response> => {
        const entry = { url, init };
        sent.push(entry);
        return answer(entry);
      },
      csrf: () => ({ "X-XSRF-TOKEN": "token-1" }),
      limits: { maxResultBytes: 1024, callTimeoutMs: 30_000 },
      scheduler: {
        timeout: (callback: VoidFunction): number => {
          const handle = next++;
          timers.set(handle, callback);
          return handle;
        },
        clearTimeout: (handle: number): void => {
          timers.delete(handle);
        },
      },
      refresh: (): Promise<void> => {
        refreshes.push(refreshes.length + 1);
        return Promise.resolve();
      },
    },
  };
}

describe("a server tool call", () => {
  it("sends the session and the CSRF token with the tool-call header", async () => {
    const run = callOptions(() =>
      Promise.resolve(
        new Response('{"id":7}', { status: 201, headers: { "content-type": "application/json" } }),
      ),
    );
    const result = await callServerTool(TOOL, { title: "Hello" }, run.options);
    expect(result).toEqual({
      content: [{ type: "text", text: '{"id":7}' }],
      structuredContent: { id: 7 },
      isError: false,
    });
    expect(run.sent).toHaveLength(1);
    const sent = run.sent[0];
    expect(sent?.url).toBe("https://app.example.test/posts");
    expect(sent?.init.method).toBe("POST");
    expect(sent?.init.credentials).toBe("same-origin");
    expect(sent?.init.redirect).toBe("manual");
    expect(sent?.init.cache).toBe("no-store");
    expect(sent?.init.body).toBe('{"title":"Hello"}');
    const headers = new Headers(sent?.init.headers);
    expect(headers.get("suprnova-tool-call")).toBe("posts.create");
    expect(headers.get("x-xsrf-token")).toBe("token-1");
    expect(headers.get("accept")).toBe("application/json");
    expect(headers.get("content-type")).toBe("application/json");
  });

  it("sends a GET tool's arguments in the query and no body", async () => {
    const run = callOptions(() =>
      Promise.resolve(new Response("{}", { headers: { "content-type": "application/json" } })),
    );
    await callServerTool(READ, { note: "n/7", format: "short" }, run.options);
    const sent = run.sent[0];
    expect(sent?.url).toBe("https://app.example.test/notes/n%2F7?note=n%2F7&format=short");
    expect(sent?.init.method).toBe("GET");
    expect(sent?.init.body).toBeUndefined();
    expect(new Headers(sent?.init.headers).get("content-type")).toBeNull();
  });

  it("refreshes the page after a write tool the server answered", async () => {
    const run = callOptions(() => Promise.resolve(new Response(null, { status: 500 })));
    const result = await callServerTool(TOOL, {}, run.options);
    expect(result.isError).toBe(true);
    expect(run.refreshes).toEqual([1]);
  });

  it("does not refresh after a read-only tool", async () => {
    const run = callOptions(() =>
      Promise.resolve(new Response("{}", { headers: { "content-type": "application/json" } })),
    );
    await callServerTool(READ, { note: "1" }, run.options);
    expect(run.refreshes).toEqual([]);
  });

  it("does not refresh when the request never reached the server", async () => {
    const run = callOptions(() => Promise.reject(new TypeError("Failed to fetch")));
    const result = await callServerTool(TOOL, {}, run.options);
    expect(result).toEqual(failureResult({ kind: "network" }));
    expect(run.refreshes).toEqual([]);
  });

  it("refuses an input that is not an object before sending anything", async () => {
    const run = callOptions(() => Promise.resolve(new Response("{}")));
    const result = await callServerTool(TOOL, [1, 2], run.options);
    expect(result.isError).toBe(true);
    expect(run.sent).toHaveLength(0);
  });

  it("refuses a bad path argument before sending anything", async () => {
    const run = callOptions(() => Promise.resolve(new Response("{}")));
    const result = await callServerTool(READ, { note: ".." }, run.options);
    expect(result.isError).toBe(true);
    expect(run.sent).toHaveLength(0);
  });

  it("stops a call that outlives its timer", async () => {
    let signal: AbortSignal | null = null;
    const run = callOptions(
      (sent) =>
        new Promise<Response>((_resolve, reject) => {
          signal = sent.init.signal ?? null;
          signal?.addEventListener("abort", () => {
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    );
    const pending = callServerTool(TOOL, {}, run.options);
    await vi.waitFor(() => {
      expect(run.sent).toHaveLength(1);
    });
    for (const fire of run.timers.values()) fire();
    expect(await pending).toEqual(failureResult({ kind: "timeout", callTimeoutMs: 30_000 }));
    expect((signal as AbortSignal | null)?.aborted).toBe(true);
    expect(run.refreshes).toEqual([]);
  });

  it("rejects with AbortError when the agent aborts", async () => {
    const run = callOptions(
      (sent) =>
        new Promise<Response>((_resolve, reject) => {
          sent.init.signal?.addEventListener("abort", () => {
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    );
    const controller = new AbortController();
    const pending = callServerTool(TOOL, {}, { ...run.options, signal: controller.signal });
    await vi.waitFor(() => {
      expect(run.sent).toHaveLength(1);
    });
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  });

  it("cuts a body at the bound while it streams", async () => {
    const chunk = new TextEncoder().encode("x".repeat(600));
    let pulled = 0;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulled += 1;
        controller.enqueue(chunk);
        if (pulled > 100) controller.close();
      },
    });
    const run = callOptions(() =>
      Promise.resolve(new Response(stream, { headers: { "content-type": "application/json" } })),
    );
    const result = await callServerTool(READ, { note: "1" }, run.options);
    expect(result).toEqual({
      content: [
        { type: "text", text: "The tool's result is larger than the 1024 bytes the page returns." },
      ],
      isError: true,
    });
    expect(pulled).toBeLessThan(10);
  });
});

// -------------------------------------------------- the whole runtime ----

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

function identity(seed: number, length: number): string {
  return Buffer.from(Uint8Array.from({ length }, (_, index) => (seed + index) % 256)).toString(
    "base64url",
  );
}

function encoded(value: JsonValue): string {
  return Buffer.from(canonicalize(value), "utf8").toString("base64url");
}

/** One plain counter island keyed `key`, as an instance or a public seed. */
function island(document: RuntimeDocument, key: string, seed: boolean): RuntimeElement {
  const slot = `counter-${key}`;
  const instance = identity(32, 16);
  const body: Record<string, JsonValue> = seed
    ? { component: { name: "tests.counter" }, form: "seed", slot, state: { count: 1 } }
    : {
        component: { name: "tests.counter" },
        form: "instance",
        instance_id: instance,
        revision: "1",
        slot,
        state: { count: 1 },
      };
  const attributes: Record<string, string> = {
    "data-suprnova-live-component": "tests.counter",
    "data-suprnova-live-contract": "1",
    "data-suprnova-live-document-key": key,
    "data-suprnova-live-island": "",
    "data-suprnova-live-lazy-complete": "true",
    // Mounted for protocol 1, as a component that declares no protocol 2
    // feature is: its fresh render still goes out under protocol 2.
    "data-suprnova-live-protocol-min": "1",
    "data-suprnova-live-revision": seed ? "0" : "1",
    "data-suprnova-live-root": slot,
    "data-suprnova-live-slot": slot,
    "data-suprnova-live-snapshot": encoded({ body, signature: identity(200, 32) }),
    "data-suprnova-live-snapshot-kind": seed ? "seed" : "instance",
  };
  if (!seed) attributes["data-suprnova-live-instance"] = instance;
  return new RuntimeElement(document, "div", attributes);
}

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
    const entry = this.registered.find(
      (candidate) => candidate.tool.name === name && candidate.signal?.aborted !== true,
    );
    if (entry === undefined) throw new Error(`no live registration of ${name}`);
    return entry.tool.execute(input);
  }
}

interface RuntimeRequest {
  readonly url: string;
  readonly init: RequestInit;
  respond(response: Response): void;
}

function page(element: string | null, seeds: readonly boolean[] = [false, true]) {
  const document = new RuntimeDocument();
  const context = new FakeModelContext();
  document.modelContext = context;
  if (element !== null) {
    const script = new RuntimeElement(document, "script", {
      id: SERVER_TOOLS_ELEMENT_ID,
      type: "application/json",
    });
    script.append(new RuntimeNode(3, document, element));
    document.head.append(script);
  }
  seeds.forEach((seed, index) => {
    document.body.append(
      island(document, seed ? `seed${String(index)}` : `main${String(index)}`, seed),
    );
  });
  const requests: RuntimeRequest[] = [];
  const fetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> =>
    new Promise<Response>((resolve) => {
      requests.push({
        url: input instanceof URL ? input.href : typeof input === "string" ? input : input.url,
        init: init ?? {},
        respond: resolve,
      });
    });
  const timers: VoidFunction[] = [];
  const scheduler = {
    microtask: (callback: VoidFunction) => {
      queueMicrotask(callback);
    },
    animationFrame: (callback: FrameRequestCallback) => {
      timers.push(() => {
        callback(0);
      });
      return timers.length;
    },
    cancelAnimationFrame: () => undefined,
    timeout: (callback: VoidFunction) => {
      timers.push(callback);
      return timers.length;
    },
    clearTimeout: () => undefined,
  };
  const ports: RuntimePorts = {
    clock: { now: () => 1_000 },
    connectivity: { isOnline: () => true },
    randomness: {
      randomBytes: (length) => Uint8Array.from({ length }, (_, index) => (index * 7 + 3) % 256),
    },
    transport: { fetch },
    navigation: { assign: () => undefined, replace: () => undefined, reload: () => undefined },
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
  const runtime = new DocumentRuntime(
    document as unknown as Document,
    runtimeConfig(),
    sink,
    ports,
    new EffectRegistry({ diagnostics: sink, scheduler }),
    new RuntimeCallRegistry({ diagnostics: sink, scheduler }),
    new IdiomorphAdapter(),
  );
  runtime.start();
  return { document, context, requests, runtime, diagnostics };
}

beforeEach(() => {
  stubDomGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the runtime's server tools", () => {
  it("registers the element's tools, and removes them on suspend and dispose", () => {
    const { context, runtime } = page(payload());
    expect(context.live()).toEqual(["notes.show", "posts.create"]);
    const registered = context.registered.find((entry) => entry.tool.name === "posts.create");
    expect(registered?.tool.title).toBe("Create a post");
    expect(registered?.tool.annotations).toEqual({
      readOnlyHint: false,
      untrustedContentHint: false,
      consequentialHint: false,
    });
    runtime.suspend();
    expect(context.live()).toEqual([]);
    runtime.resume();
    expect(context.live()).toEqual(["notes.show", "posts.create"]);
    runtime.dispose();
    expect(context.live()).toEqual([]);
  });

  it("registers nothing for an invalid element, and says so", () => {
    const { context, diagnostics } = page('{"v":2}');
    expect(context.live()).toEqual([]);
    expect(diagnostics).toContainEqual({
      code: "agent_tool_rejected",
      severity: "warning",
      phase: "discovery",
      detailCode: "invalid_shape",
    });
  });

  it("registers nothing without the element", () => {
    const { context } = page(null);
    expect(context.live()).toEqual([]);
  });

  it("calls the tool's route, then renders every island fresh before it answers", async () => {
    const { context, requests } = page(payload());
    const result = context.call("posts.create", { title: "Hello" });
    await vi.waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    const call = requests[0];
    expect(call?.url).toBe("https://app.example.test/posts");
    expect(new Headers(call?.init.headers).get("suprnova-tool-call")).toBe("posts.create");
    let settled = false;
    void result.then(() => {
      settled = true;
    });
    call?.respond(
      new Response('{"id":1}', { status: 201, headers: { "content-type": "application/json" } }),
    );
    // Both islands ask for a fresh render: the instance with its snapshot,
    // the public seed promoted with a browser nonce.
    await vi.waitFor(() => {
      expect(requests).toHaveLength(3);
    });
    const renders = requests.slice(1).map((request) => {
      const body = JSON.parse(request.init.body as string) as Record<string, unknown>;
      return {
        protocol: body["protocol_version"],
        operations: body["operations"],
        kind: (body["snapshot"] as Record<string, unknown>)["kind"],
        key: (body["extensions"] as Record<string, unknown>)["x_suprnova_live_document_key_v1"],
      };
    });
    expect(renders).toContainEqual({
      protocol: 2,
      operations: [{ kind: "fresh_render" }],
      kind: "instance",
      key: "main0",
    });
    expect(renders).toContainEqual({
      protocol: 2,
      operations: [{ kind: "fresh_render" }],
      kind: "seed_promotion",
      key: "seed1",
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(settled).toBe(false);
    for (const request of requests.slice(1)) request.respond(new Response(null, { status: 500 }));
    expect(await result).toEqual({
      content: [{ type: "text", text: '{"id":1}' }],
      structuredContent: { id: 1 },
      isError: false,
    });
  });

  it("does not render the islands after a read-only tool", async () => {
    const { context, requests } = page(payload());
    const result = context.call("notes.show", { note: "1" });
    await vi.waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    requests[0]?.respond(new Response("{}", { headers: { "content-type": "application/json" } }));
    expect((await result).isError).toBe(false);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(requests).toHaveLength(1);
  });
});
