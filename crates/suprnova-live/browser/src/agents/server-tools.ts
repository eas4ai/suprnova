/**
 * The server tools a page offers (AGENT-014): the tools the server lists
 * in the head's `<script type="application/json" id="suprnova-agent-tools">`
 * element, called from the page as the MCP endpoint calls them, and their
 * answers mapped to WebMCP results.
 *
 * The server decides which tools a page offers and lists only those its
 * viewer may call; the route checks the caller again on every call. A call
 * goes to the tool's own route on the page's origin, with the session
 * cookie, the page's CSRF token and the `Suprnova-Tool-Call` header, so it
 * meets every check the route applies. Each rule here mirrors the shared
 * cases every browser implementation reads
 * (`framework/tests/agents/fixtures/browser-calls.json`), which mirror the
 * MCP dispatcher's path rules and result mapping.
 */
import type { AgentToolAnnotations } from "../islands/metadata.js";
import type { ModelContextToolResult, WebMcpToolDefinition } from "./model-context.js";
import { TOOL_CALL_HEADER } from "../transport/fetch.js";

/** The `id` of the element that lists the page's server tools. */
export const SERVER_TOOLS_ELEMENT_ID = "suprnova-agent-tools";

const MAX_SERVER_TOOLS = 64;
const TOOL_NAME = /^[A-Za-z0-9_.-]{1,128}$/u;
const PARAM_NAME = /^[A-Za-z_][A-Za-z0-9_]{0,127}$/u;
const METHODS = new Set(["GET", "POST", "PUT", "PATCH", "DELETE"]);
const HINTS = ["consequentialHint", "debugging", "readOnlyHint", "untrustedContentHint"] as const;
/** How many invalid fields a validation result names; the rest are counted. */
const MAX_REPORTED = 20;
/** The longest field path a validation result shows, in characters. */
const MAX_PATH_CHARS = 200;
/** The longest message a validation result shows for one field, in characters. */
const MAX_MESSAGE_CHARS = 500;

/** The limits the server puts on one call. */
export interface ServerToolLimits {
  readonly maxResultBytes: number;
  readonly callTimeoutMs: number;
}

/** One piece of a tool's route path: literal text, or a parameter to fill. */
export type ServerToolPathPiece =
  Readonly<{ text: string }> | Readonly<{ param: string; optional: boolean; catchAll: boolean }>;

/** One server tool the page offers. */
export interface ServerToolDefinition extends WebMcpToolDefinition {
  readonly method: string;
  readonly path: readonly ServerToolPathPiece[];
}

/** The page's server tools and the limits of their calls. */
export interface ServerTools {
  readonly limits: ServerToolLimits;
  readonly tools: readonly ServerToolDefinition[];
}

/** The element does not hold a payload the runtime accepts. */
export class ServerToolsError extends Error {
  constructor(readonly detail: string) {
    super("server_tools_invalid");
    this.name = "ServerToolsError";
  }
}

function fail(detail: string): never {
  throw new ServerToolsError(detail);
}

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const own = Object.keys(value).sort();
  return own.length === keys.length && [...keys].sort().every((key, index) => own[index] === key);
}

function positiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function pathPiece(value: unknown, first: boolean): ServerToolPathPiece {
  if (!plainObject(value)) return fail("path");
  if (exactKeys(value, ["text"]) && typeof value["text"] === "string") {
    if (first && !value["text"].startsWith("/")) return fail("path");
    return Object.freeze({ text: value["text"] });
  }
  if (
    !first &&
    exactKeys(value, ["catchAll", "optional", "param"]) &&
    typeof value["param"] === "string" &&
    PARAM_NAME.test(value["param"]) &&
    typeof value["optional"] === "boolean" &&
    typeof value["catchAll"] === "boolean"
  ) {
    return Object.freeze({
      param: value["param"],
      optional: value["optional"],
      catchAll: value["catchAll"],
    });
  }
  return fail("path");
}

function annotations(value: unknown): AgentToolAnnotations {
  if (
    !plainObject(value) ||
    !exactKeys(value, HINTS) ||
    !HINTS.every((hint) => typeof value[hint] === "boolean")
  ) {
    return fail("annotations");
  }
  return Object.freeze({
    readOnlyHint: value["readOnlyHint"] as boolean,
    untrustedContentHint: value["untrustedContentHint"] as boolean,
    consequentialHint: value["consequentialHint"] as boolean,
    debugging: value["debugging"] as boolean,
  });
}

function tool(value: unknown, names: Set<string>): ServerToolDefinition {
  if (!plainObject(value)) return fail("tool");
  const allowed = new Set([
    "annotations",
    "description",
    "inputSchema",
    "method",
    "name",
    "path",
    "title",
  ]);
  const { name, title, description, inputSchema, method, path } = value;
  if (
    Object.keys(value).some((key) => !allowed.has(key)) ||
    typeof name !== "string" ||
    !TOOL_NAME.test(name) ||
    names.has(name) ||
    (title !== undefined && (typeof title !== "string" || title.trim().length === 0)) ||
    typeof description !== "string" ||
    description.trim().length === 0 ||
    !plainObject(inputSchema) ||
    typeof method !== "string" ||
    !METHODS.has(method) ||
    !Array.isArray(path) ||
    path.length === 0
  ) {
    return fail("tool");
  }
  names.add(name);
  return Object.freeze({
    name,
    ...(typeof title === "string" ? { title } : {}),
    description,
    inputSchema: Object.freeze(inputSchema),
    annotations: annotations(value["annotations"]),
    method,
    path: Object.freeze(path.map((piece: unknown, index) => pathPiece(piece, index === 0))),
  });
}

/**
 * Read the element's payload, strictly: exactly `v`, `limits` and `tools`;
 * `v` is 1; both limits are positive whole numbers; 1 to 64 tools, each with
 * a unique name in the tool grammar, an optional non-blank title, a
 * non-blank description, an object input schema, the four boolean hints,
 * a method and a path of text and parameter pieces that starts with text.
 * Anything else is refused whole, so the page registers nothing it did not
 * understand.
 */
export function parseServerTools(text: string): ServerTools {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return fail("json");
  }
  if (!plainObject(parsed) || !exactKeys(parsed, ["limits", "tools", "v"]) || parsed["v"] !== 1) {
    return fail("payload");
  }
  const limits = parsed["limits"];
  const tools = parsed["tools"];
  if (
    !plainObject(limits) ||
    !exactKeys(limits, ["callTimeoutMs", "maxResultBytes"]) ||
    !positiveInteger(limits["maxResultBytes"]) ||
    !positiveInteger(limits["callTimeoutMs"]) ||
    !Array.isArray(tools) ||
    tools.length === 0 ||
    tools.length > MAX_SERVER_TOOLS
  ) {
    return fail("payload");
  }
  const names = new Set<string>();
  return Object.freeze({
    limits: Object.freeze({
      maxResultBytes: limits["maxResultBytes"],
      callTimeoutMs: limits["callTimeoutMs"],
    }),
    tools: Object.freeze(tools.map((entry: unknown) => tool(entry, names))),
  });
}

/**
 * The page's server tools, from the element in its head; `null` when the
 * page offers none. Markup in the body cannot pose as the element.
 *
 * @throws {ServerToolsError} when the element holds a payload the runtime
 * does not accept.
 */
export function readServerTools(document: Document): ServerTools | null {
  const head = document.head as Element | null;
  if (head === null) return null;
  for (const script of Array.from(head.querySelectorAll("script"))) {
    if (
      script.parentElement !== head ||
      script.getAttribute("id") !== SERVER_TOOLS_ELEMENT_ID ||
      script.getAttribute("type") !== "application/json"
    ) {
      continue;
    }
    return parseServerTools(script.textContent);
  }
  return null;
}

/** A result with `text` as its one content item. */
function textResult(text: string, isError: boolean): ModelContextToolResult {
  return Object.freeze({
    content: Object.freeze([Object.freeze({ type: "text" as const, text })]),
    isError,
  });
}

/** An error result with `text`. */
function errorResult(text: string): ModelContextToolResult {
  return textResult(text, true);
}

/** What a tool call sends, or the result of a call refused before it left. */
export type ServerCall =
  | Readonly<{ kind: "send"; url: string; body: string | null }>
  | Readonly<{ kind: "refused"; result: ModelContextToolResult }>;

function refused(reason: string): ServerCall {
  return Object.freeze({ kind: "refused", result: errorResult(`Invalid input: ${reason}.`) });
}

/** A scalar argument as text, `null` for an absent or null one, `undefined` for any other. */
function scalarText(value: unknown): string | null | undefined {
  if (value === undefined || value === null) return null;
  if (typeof value === "string") return value;
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : undefined;
  if (typeof value === "boolean") return value ? "true" : "false";
  return undefined;
}

/**
 * `text` as one path segment: every byte but the unreserved letters, digits,
 * `-`, `.`, `_` and `~` percent-encoded, and a segment of three or more dots
 * written as `%2E` repeats.
 */
function encodeSegment(text: string): string {
  if (text.length > 0 && /^\.+$/u.test(text)) return "%2E".repeat(text.length);
  return encodeURIComponent(text).replace(
    /[!'()*]/gu,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
  );
}

/**
 * The request a call of a tool at `method` and `path` sends with `args`:
 *
 * - each parameter is filled from the argument of its name, a string, a
 *   number or a boolean, each segment percent-encoded, a catch-all split on
 *   `/` with each part encoded;
 * - a value of `.` or `..`, or a catch-all part that is one, is refused: the
 *   page's URL parser would resolve it as a step of the path before the
 *   request left;
 * - an optional parameter that is absent, null or empty drops its piece and
 *   the `/` before it, and a required one is refused;
 * - a `GET` sends every scalar argument in the query in argument order, path
 *   arguments too, except a null one and a path argument that filled no
 *   segment, and refuses a structured one, since a `GET` carries no body;
 * - any other method sends the whole argument object as its JSON body.
 */
export function buildServerCall(
  method: string,
  path: readonly ServerToolPathPiece[],
  args: Readonly<Record<string, unknown>>,
): ServerCall {
  let url = "";
  const unfilled = new Set<string>();
  const inPath = new Set<string>();
  for (const piece of path) {
    if ("text" in piece) {
      url += piece.text;
      continue;
    }
    inPath.add(piece.param);
    const text = scalarText(args[piece.param]);
    if (text === undefined) {
      return refused(
        `the path parameter \`${piece.param}\` must be a string, a number or a boolean`,
      );
    }
    if (text === null || text.length === 0) {
      if (!piece.optional) return refused(`the path parameter \`${piece.param}\` is missing`);
      if (url.endsWith("/")) url = url.slice(0, -1);
      unfilled.add(piece.param);
      continue;
    }
    const parts = piece.catchAll ? text.split("/") : [text];
    if (parts.some((part) => part === "." || part === "..")) {
      return refused(
        `the path parameter \`${piece.param}\` cannot be \`.\` or \`..\`, or hold one as a part, because a browser resolves it as a step of the path`,
      );
    }
    url += parts.map(encodeSegment).join("/");
  }
  if (url.length === 0) url = "/";
  if (method !== "GET") {
    return Object.freeze({ kind: "send", url, body: JSON.stringify(args) });
  }
  const query = new URLSearchParams();
  for (const [name, value] of Object.entries(args)) {
    const text = scalarText(value);
    if (text === undefined) {
      return refused(
        `the argument \`${name}\` must be a string, a number or a boolean, because a GET tool sends its arguments in the query`,
      );
    }
    if (text === null || (inPath.has(name) && unfilled.has(name))) continue;
    query.append(name, text);
  }
  const search = query.toString();
  return Object.freeze({
    kind: "send",
    url: search.length > 0 ? `${url}?${search}` : url,
    body: null,
  });
}

/**
 * The CSRF header a call sends: `X-CSRF-TOKEN` with the page's
 * `csrf-token` meta tag, else `X-XSRF-TOKEN` with the decoded `XSRF-TOKEN`
 * cookie, else none.
 */
export function csrfHeader(meta: string | null, cookie: string): Record<string, string> {
  if (meta !== null && meta.length > 0) return { "X-CSRF-TOKEN": meta };
  for (const entry of cookie.split(";")) {
    const trimmed = entry.trim();
    const separator = trimmed.indexOf("=");
    if (separator < 0 || trimmed.slice(0, separator) !== "XSRF-TOKEN") continue;
    const raw = trimmed.slice(separator + 1);
    try {
      return { "X-XSRF-TOKEN": decodeURIComponent(raw) };
    } catch {
      return { "X-XSRF-TOKEN": raw };
    }
  }
  return {};
}

/** The page's CSRF header, read when a call is made. */
export function pageCsrfHeader(document: Document): Record<string, string> {
  let meta: string | null = null;
  for (const element of Array.from(document.querySelectorAll("meta"))) {
    if (element.getAttribute("name") === "csrf-token") {
      meta = element.getAttribute("content");
      break;
    }
  }
  const cookie: unknown = Reflect.get(document, "cookie");
  return csrfHeader(meta, typeof cookie === "string" ? cookie : "");
}

/** Why a call ended without an answer to map. */
export type ServerCallFailure =
  | Readonly<{ kind: "network" }>
  | Readonly<{ kind: "timeout"; callTimeoutMs: number }>
  | Readonly<{ kind: "too_large"; maxResultBytes: number }>;

/** The result of a call that ended without an answer to map. */
export function failureResult(failure: ServerCallFailure): ModelContextToolResult {
  switch (failure.kind) {
    case "network":
      return errorResult("The tool could not be reached.");
    case "timeout":
      return errorResult(
        `The tool did not finish within ${String(failure.callTimeoutMs / 1000)} seconds and was stopped.`,
      );
    case "too_large":
      return errorResult(
        `The tool's result is larger than the ${String(failure.maxResultBytes)} bytes the page returns.`,
      );
  }
}

/** The parts of a fetch `Response` the mapping reads. */
export interface ServerResponseLike {
  readonly type: string;
  readonly status: number;
  readonly headers: { get(name: string): string | null };
  readonly body: ReadableStream<Uint8Array> | null;
}

/** Whether `contentType` is JSON: `application/json` or a `+json` type. */
function isJson(contentType: string | null): boolean {
  if (contentType === null) return false;
  const media = (contentType.split(";")[0] ?? "").trim().toLowerCase();
  return media === "application/json" || media.endsWith("+json");
}

/** `text` on one line, its control characters spaces, cut to `max` characters. */
function oneLine(text: string, max: number): string {
  const characters = Array.from(text);
  const kept = characters
    .slice(0, max)
    .map((character) => (/\p{Cc}/u.test(character) ? " " : character))
    .join("");
  return characters.length > max ? `${kept}...` : kept;
}

/** `text` cut to at most `maxBytes` UTF-8 bytes, at a character, with an ellipsis. */
function bounded(text: string, maxBytes: number): string {
  const encoder = new TextEncoder();
  if (encoder.encode(text).length <= maxBytes) return text;
  const budget = Math.max(maxBytes - 3, 0);
  let out = "";
  let used = 0;
  for (const character of text) {
    const size = encoder.encode(character).length;
    if (used + size > budget) break;
    out += character;
    used += size;
  }
  return `${out}...`;
}

/** The text of one message of a validation report. */
function messageText(message: unknown): string {
  return typeof message === "string" ? message : JSON.stringify(message);
}

/** A validation failure naming each field, bounded as the MCP endpoint bounds it. */
function validationResult(
  errors: Readonly<Record<string, unknown>>,
  maxBytes: number,
): ModelContextToolResult {
  const entries = Object.entries(errors);
  let text = "The input is not valid:";
  for (const [field, messages] of entries.slice(0, MAX_REPORTED)) {
    const words = Array.isArray(messages)
      ? messages.map(messageText)
      : [typeof messages === "string" ? messages : JSON.stringify(messages)];
    text += `\n- ${oneLine(field, MAX_PATH_CHARS)}: ${oneLine(words.join(" "), MAX_MESSAGE_CHARS)}`;
  }
  if (entries.length > MAX_REPORTED) {
    text += `\n(${String(entries.length - MAX_REPORTED)} more fields are not valid.)`;
  }
  return errorResult(bounded(text, maxBytes));
}

/** The `message` of an error body, or `fallback`. */
function messageOf(body: unknown, fallback: string): string {
  const message = plainObject(body) ? body["message"] : undefined;
  return typeof message === "string" && message.length > 0 ? message : fallback;
}

/** The result of an answer whose whole body, within the bound, is `text`. */
function answerResult(
  status: number,
  contentType: string | null,
  text: string,
  maxBytes: number,
): ModelContextToolResult {
  let json: unknown;
  let parsed = false;
  if (isJson(contentType)) {
    try {
      json = JSON.parse(text);
      parsed = true;
    } catch {
      parsed = false;
    }
  }
  if (status >= 200 && status <= 299) {
    if (text.length === 0) return Object.freeze({ content: Object.freeze([]), isError: false });
    if (!parsed) return errorResult("The tool answered with a body that is not JSON.");
    return Object.freeze({
      content: Object.freeze([Object.freeze({ type: "text" as const, text })]),
      structuredContent: json,
      isError: false,
    });
  }
  if (status >= 300 && status <= 399) {
    return errorResult("The tool answered with a redirect, which a tool call cannot follow.");
  }
  const body = parsed ? json : undefined;
  if (status === 422) {
    const errors = plainObject(body) ? body["errors"] : undefined;
    return plainObject(errors)
      ? validationResult(errors, maxBytes)
      : errorResult(messageOf(body, "The input is not valid."));
  }
  if (status === 419) {
    return errorResult("The page's session expired: reload the page, then call the tool again.");
  }
  if (status === 429) return errorResult("Too many calls: wait before calling the tool again.");
  if (status >= 500) return errorResult("The tool failed.");
  if (status === 401) return errorResult(messageOf(body, "The tool needs a signed-in user."));
  if (status === 403) return errorResult(messageOf(body, "This action is unauthorized."));
  if (status === 404) return errorResult(messageOf(body, "Not found."));
  return errorResult(messageOf(body, "The tool refused the call."));
}

/**
 * Read `body` whole, unless it holds more than `maxBytes` bytes, which
 * stops the read: `null` then.
 */
async function boundedBody(
  body: ReadableStream<Uint8Array> | null,
  maxBytes: number,
): Promise<string | null> {
  if (body === null) return "";
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > maxBytes) {
      await reader.cancel().catch(() => undefined);
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder("utf-8").decode(bytes);
}

/**
 * The result of the tool's answer: a redirect the page did not follow, a
 * body over the bound whatever its status, or the status, content type and
 * body mapped as the MCP endpoint maps a route's answer.
 */
export async function resultOfResponse(
  response: ServerResponseLike,
  maxResultBytes: number,
): Promise<ModelContextToolResult> {
  if (response.type === "opaqueredirect") {
    return errorResult("The tool answered with a redirect, which a tool call cannot follow.");
  }
  const text = await boundedBody(response.body, maxResultBytes);
  if (text === null) return failureResult({ kind: "too_large", maxResultBytes });
  return answerResult(response.status, response.headers.get("content-type"), text, maxResultBytes);
}

/** The timer a call is bounded by. */
export interface ServerCallScheduler {
  timeout(callback: VoidFunction, delay: number): number;
  clearTimeout(handle: number): void;
}

/** What one call needs from the page. */
export interface ServerToolCallOptions {
  /** The page's origin, which every call stays on. */
  readonly origin: string;
  readonly fetch: (url: string, init: RequestInit) => Promise<Response>;
  /** The CSRF header, read at the call. */
  readonly csrf: () => Record<string, string>;
  readonly limits: ServerToolLimits;
  readonly scheduler: ServerCallScheduler;
  /** The agent's signal: an abort rejects the call with `AbortError`. */
  readonly signal?: AbortSignal;
  /** Refresh the page's data after a write tool the server answered. */
  readonly refresh: () => Promise<void>;
}

/** Whether the agent aborted, read afresh at each await. */
function aborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

function abortError(): DOMException {
  return new DOMException("The tool call was aborted.", "AbortError");
}

/**
 * Call `tool` with `input` as the page: build the request, check it stays
 * on the page's origin before any credential is attached, send it with the
 * session cookie, the CSRF header and the tool-call header, follow no
 * redirect, and map the answer, bounded by the call timer and the result
 * bound. After a tool not marked read-only, once the server answered, the
 * page's data is refreshed before the call resolves, so the agent's next
 * read of the page sees what it did; the refresh waits at most the call
 * timer, and its outcome does not change the result.
 */
export async function callServerTool(
  tool: ServerToolDefinition,
  input: unknown,
  options: ServerToolCallOptions,
): Promise<ModelContextToolResult> {
  if (aborted(options.signal)) throw abortError();
  if (!plainObject(input)) return errorResult("Invalid input: the input must be an object.");
  const call = buildServerCall(tool.method, tool.path, input);
  if (call.kind === "refused") return call.result;
  let target: URL;
  try {
    target = new URL(call.url, options.origin);
  } catch {
    return errorResult("Invalid input: the tool's address is not valid.");
  }
  if (target.origin !== options.origin) {
    return errorResult("Invalid input: the tool's address leaves the page's origin.");
  }
  const headers: Record<string, string> = {
    Accept: "application/json",
    [TOOL_CALL_HEADER]: tool.name,
    ...options.csrf(),
  };
  if (call.body !== null) headers["Content-Type"] = "application/json";
  const controller = new AbortController();
  // What ended the request early, set by the timer and the agent's signal.
  const ended = { timedOut: false, agentAborted: false };
  const onAbort = (): void => {
    ended.agentAborted = true;
    controller.abort();
  };
  options.signal?.addEventListener("abort", onAbort, { once: true });
  const timer = options.scheduler.timeout(() => {
    ended.timedOut = true;
    controller.abort();
  }, options.limits.callTimeoutMs);
  let answered = false;
  let result: ModelContextToolResult;
  try {
    const init: RequestInit = {
      method: tool.method,
      headers,
      credentials: "same-origin",
      redirect: "manual",
      cache: "no-store",
      signal: controller.signal,
    };
    if (call.body !== null) init.body = call.body;
    const response = await options.fetch(target.href, init);
    answered = true;
    result = await resultOfResponse(response, options.limits.maxResultBytes);
  } catch {
    if (ended.agentAborted) throw abortError();
    result = ended.timedOut
      ? failureResult({ kind: "timeout", callTimeoutMs: options.limits.callTimeoutMs })
      : failureResult({ kind: "network" });
  } finally {
    options.scheduler.clearTimeout(timer);
    options.signal?.removeEventListener("abort", onAbort);
  }
  if (answered && !tool.annotations.readOnlyHint) {
    await refreshWithin(options);
    if (aborted(options.signal)) throw abortError();
  }
  return result;
}

/** Run the refresh, waiting for it at most the call timer. */
async function refreshWithin(options: ServerToolCallOptions): Promise<void> {
  let expire: VoidFunction = () => undefined;
  const expired = new Promise<void>((resolve) => {
    expire = resolve;
  });
  const handle = options.scheduler.timeout(() => {
    expire();
  }, options.limits.callTimeoutMs);
  try {
    await Promise.race([options.refresh().catch(() => undefined), expired]);
  } finally {
    options.scheduler.clearTimeout(handle);
  }
}
