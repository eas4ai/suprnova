/**
 * WebMCP registration of a Live island's tool actions (AGENT-012).
 *
 * The page registers each tool the island root lists with
 * `document.modelContext.registerTool`, each with its own abort signal, and
 * aborts it when the island leaves the page, the document is suspended into
 * the back/forward cache, or the runtime is disposed. A browser without
 * WebMCP registers nothing: the runtime never depends on the API.
 */
import type { AgentToolDefinition } from "../islands/metadata.js";

/** What a WebMCP tool's `execute` resolves to. */
export interface ModelContextToolResult {
  readonly content: readonly Readonly<{ type: "text"; text: string }>[];
  readonly structuredContent: Readonly<Record<string, unknown>>;
  readonly isError: boolean;
}

/** The client WebMCP passes `execute`, when it passes one. */
export interface ModelContextClient {
  readonly signal?: AbortSignal;
}

/** The tool a page registers, as the WebMCP draft defines it. */
export interface ModelContextTool {
  readonly name: string;
  readonly title?: string;
  readonly description: string;
  readonly inputSchema: Readonly<Record<string, unknown>>;
  readonly annotations: Readonly<Record<string, boolean>>;
  readonly execute: (
    input: unknown,
    client?: ModelContextClient,
  ) => Promise<ModelContextToolResult>;
}

/** The part of `document.modelContext` the runtime uses. */
export interface ModelContextLike {
  /** Returns a promise in the draft; a promise that rejects is a refusal. */
  registerTool(tool: ModelContextTool, options?: Readonly<{ signal?: AbortSignal }>): unknown;
}

/**
 * The page's model context, when the browser exposes WebMCP: a
 * `modelContext` on the document or the navigator with a `registerTool`
 * function.
 */
export function modelContextOf(document: Document): ModelContextLike | null {
  const candidates: unknown[] = [
    Reflect.get(document, "modelContext"),
    Reflect.get(Reflect.get(document.defaultView ?? {}, "navigator") ?? {}, "modelContext"),
  ];
  for (const candidate of candidates) {
    if (
      typeof candidate === "object" &&
      candidate !== null &&
      typeof Reflect.get(candidate, "registerTool") === "function"
    ) {
      return candidate as ModelContextLike;
    }
  }
  return null;
}

/** Why a registration did not take. */
export type RegistrationFailure = "rejected" | "unavailable";

/**
 * Register `tools` with `context`, each under its own controller, and
 * return the function that aborts every one of them. `execute` answers a
 * call of one tool; `onFailure` hears of a registration the browser refused,
 * a duplicate or invalid name above all.
 */
export function registerAgentTools(
  context: ModelContextLike,
  tools: readonly AgentToolDefinition[],
  execute: (
    tool: AgentToolDefinition,
    input: unknown,
    client?: ModelContextClient,
  ) => Promise<ModelContextToolResult>,
  onFailure: (failure: RegistrationFailure) => void,
): VoidFunction {
  const controllers: AbortController[] = [];
  for (const tool of tools) {
    const controller = new AbortController();
    controllers.push(controller);
    const definition: ModelContextTool = Object.freeze({
      name: tool.name,
      ...(tool.title === undefined ? {} : { title: tool.title }),
      description: tool.description,
      inputSchema: tool.inputSchema,
      annotations: Object.freeze({ ...tool.annotations }),
      execute: (input: unknown, client?: ModelContextClient) => execute(tool, input, client),
    });
    try {
      const registered = context.registerTool(definition, { signal: controller.signal });
      if (registered instanceof Promise) {
        registered.catch(() => {
          // An abort before the registration settled is the page's own
          // removal, not a refusal.
          if (!controller.signal.aborted) onFailure("rejected");
        });
      }
    } catch {
      onFailure("rejected");
    }
  }
  let aborted = false;
  return () => {
    if (aborted) return;
    aborted = true;
    for (const controller of controllers) controller.abort();
  };
}
