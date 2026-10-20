// Agent tools on a kit page, for WebMCP's `document.modelContext`
// (AGENT-014, AGENT-015): registering a tool for as long as the page or a
// component wants it, calling the server tools a page lists, and turning
// a kit form into a tool.
//
// `inertia.ts` registers each page's server tools and `index.ts` holds the
// kit's hooks over this file. It imports no UI framework and touches the
// browser only when a tool registers or runs, never at import, so the
// server render can import it. A browser without WebMCP registers nothing,
// and the page works as it always did.

import type {} from 'webmcp-types'

import { csrfHeaders } from '../stream/core'

/** One text block of a tool result. */
export type ToolText = { type: 'text'; text: string }

/** What a WebMCP tool call resolves to, carrying each answer once. */
export type ToolResult = {
  content?: ToolText[]
  /** The answer as data, when it is JSON. */
  structuredContent?: unknown
  /** The call failed; the answer says why, for the agent to read. */
  isError: boolean
}

/** What WebMCP hands `execute` beside the input: the agent's abort signal. */
export type ToolClient = Partial<WebMCP.ToolExecuteCallbackOptions>

/**
 * The four hints every registration carries, from the tool's markings.
 * `debugging` marks a tool meant for debugging and developer tooling, not
 * end users.
 */
export type ToolAnnotations = Required<WebMCP.ToolAnnotations>

/** A JSON Schema whose root is an object, as a tool's input must be. */
export type ToolInputSchema = { readonly type: 'object'; readonly [key: string]: unknown }

/** One tool as the page registers it with `document.modelContext`. */
export type ToolDefinition = Omit<WebMCP.ModelContextTool, 'inputSchema' | 'annotations' | 'execute'> & {
  inputSchema: ToolInputSchema
  annotations: ToolAnnotations
  execute: (input: unknown, client?: ToolClient) => Promise<ToolResult>
}

/** The part of `document.modelContext` a page uses, checked against WebMCP. */
export type ModelContextLike = Pick<WebMCP.ModelContext, 'registerTool'>

/** The most tools one page lists. */
export const MAX_PAGE_TOOLS = 64

/** What the server tools' route answers are cut at when a page names no limit. */
export const DEFAULT_MAX_RESULT_BYTES = 8 * 1024 * 1024

/** The header that marks a request as an agent's call of the route's tool. */
export const TOOL_CALL_HEADER = 'Suprnova-Tool-Call'

/**
 * The header that marks a form a form tool submits: the server's
 * `suprnova::agents::FORM_TOOL_HEADER`. A consent page refuses it, so an
 * agent filling the approval or claim form can neither approve nor claim.
 */
export const FORM_TOOL_HEADER = 'Suprnova-Form-Tool'

/** A tool name the framework accepts (AGENT-001). */
const TOOL_NAME = /^[A-Za-z0-9_]{1,64}$/u

const LEFT = "The tool has left the page: call again once the page lists it."

/** The page's model context, when the browser exposes WebMCP. */
export function modelContextOf(doc: Document): ModelContextLike | null {
  const usable = (candidate: unknown): candidate is ModelContextLike =>
    typeof candidate === 'object' &&
    candidate !== null &&
    typeof Reflect.get(candidate, 'registerTool') === 'function'
  // The draft's home is the document; the navigator's is read only where
  // the document has none, since Chromium warns that it is deprecated.
  const onDocument: unknown = Reflect.get(doc, 'modelContext')
  if (usable(onDocument)) {
    return onDocument
  }
  const navigator: unknown = Reflect.get(doc.defaultView ?? {}, 'navigator')
  const onNavigator: unknown =
    typeof navigator === 'object' && navigator !== null
      ? Reflect.get(navigator, 'modelContext')
      : undefined
  return usable(onNavigator) ? onNavigator : null
}

/** A tool error the agent reads. */
export function errorResult(message: string): ToolResult {
  return { content: [{ type: 'text', text: message }], isError: true }
}

function invalidInput(reason: string): ToolResult {
  return errorResult(`Invalid input: ${reason}.`)
}

function abortError(): DOMException {
  return new DOMException('The tool call was aborted.', 'AbortError')
}

/** Whether the agent aborted its call; read fresh, since it can change across an `await`. */
function aborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true
}

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Why `tool` cannot be registered, or `null`. */
function definitionProblem(tool: ToolDefinition): string | null {
  if (!TOOL_NAME.test(tool.name)) {
    return 'a tool name is 1 to 64 ASCII letters, digits and `_`'
  }
  if (typeof tool.description !== 'string' || tool.description.trim() === '') {
    return 'it has no description'
  }
  if (tool.title !== undefined && (typeof tool.title !== 'string' || tool.title.trim() === '')) {
    return 'its title is empty'
  }
  if (!plainObject(tool.inputSchema) || tool.inputSchema.type !== 'object') {
    return "its input schema's root is not an object schema"
  }
  return null
}

/**
 * One tool on the page. It registers under a fresh controller each time it
 * starts and keeps its definition while the page sits in the back/forward
 * cache.
 *
 * A tool that leaves the page unregisters at once, even with a call
 * pending: Chromium still hands that call's answer to the agent, and an
 * Inertia visit keeps the document, so nothing ends the call. The Live
 * runtime holds a leaving island's tools for 250 ms because it reloads the
 * document; a kit page does not, which the kits' browser suite checks with
 * a form whose submit takes it off the page.
 */
class Registration {
  readonly tool: ToolDefinition
  #controller: AbortController | null = null
  #gone = false

  constructor(tool: ToolDefinition) {
    this.tool = tool
  }

  /** Register with the browser, unless the tool already left or is live. */
  start(): void {
    const context = modelContextOf(document)
    if (this.#gone || this.#controller !== null || context === null) {
      return
    }
    const controller = new AbortController()
    this.#controller = controller
    const definition: ToolDefinition = {
      name: this.tool.name,
      ...(this.tool.title === undefined ? {} : { title: this.tool.title }),
      description: this.tool.description,
      inputSchema: this.tool.inputSchema,
      annotations: { ...this.tool.annotations },
      // A call through a registration that left is refused.
      execute: (input, client) =>
        this.#gone ? Promise.resolve(errorResult(LEFT)) : this.tool.execute(input, client),
    }
    const refused = (error: unknown) => {
      // An abort before the registration settled is the page's own removal.
      if (!controller.signal.aborted) {
        console.error(`The browser refused the page tool "${this.tool.name}".`, error)
      }
    }
    try {
      const registered = context.registerTool(definition, { signal: controller.signal })
      if (registered instanceof Promise) {
        registered.catch(refused)
      }
    } catch (error) {
      refused(error)
    }
  }

  /** Unregister for now, keeping the definition: the page was suspended. */
  stop(): void {
    const controller = this.#controller
    this.#controller = null
    controller?.abort()
  }

  /** The tool leaves the page for good. */
  leave(): void {
    this.#gone = true
    this.stop()
  }
}

/** The tools on the page, by name: names are unique within a page. */
const registrations = new Map<string, Registration>()
let lifecycleInstalled = false

/**
 * Suspend every tool when the page enters the back/forward cache and
 * register them again when it comes back, as the Live runtime does.
 */
function installLifecycle(): void {
  if (lifecycleInstalled) {
    return
  }
  lifecycleInstalled = true
  window.addEventListener('pagehide', (event) => {
    if (event.persisted) {
      for (const registration of registrations.values()) {
        registration.stop()
      }
    }
  })
  window.addEventListener('pageshow', (event) => {
    if (event.persisted) {
      for (const registration of registrations.values()) {
        registration.start()
      }
    }
  })
}

/**
 * Register `tool` with the page's model context and return the function
 * that removes it. A name another tool on the page holds, or one outside
 * the framework's grammar, is not registered and is reported with
 * `console.error`.
 */
export function registerTool(tool: ToolDefinition): () => void {
  if (typeof document === 'undefined' || modelContextOf(document) === null) {
    return () => undefined
  }
  const problem = definitionProblem(tool)
  if (problem !== null) {
    console.error(`The page tool "${tool.name}" was not registered: ${problem}.`)
    return () => undefined
  }
  if (registrations.has(tool.name)) {
    console.error(
      `The page tool "${tool.name}" was not registered: another tool on the page has that name.`,
    )
    return () => undefined
  }
  installLifecycle()
  const registration = new Registration(tool)
  registrations.set(tool.name, registration)
  registration.start()
  return () => {
    registration.leave()
    if (registrations.get(tool.name) === registration) {
      registrations.delete(tool.name)
    }
  }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/** The most fields a validation failure lists. */
const MAX_REPORTED_FIELDS = 20
/** The longest field name a validation failure shows. */
const MAX_FIELD_CHARS = 200
/** The longest message a validation failure shows for one field. */
const MAX_MESSAGE_CHARS = 500

/** Whether a code point is a control character (Unicode's `Cc`). */
function isControl(char: string): boolean {
  const code = char.codePointAt(0) ?? 0
  return code <= 0x1f || (code >= 0x7f && code <= 0x9f)
}

/** `text` on one line, cut to `maxChars` characters with an ellipsis. */
function oneLine(text: string, maxChars: number): string {
  const chars = Array.from(text)
  const line = chars
    .slice(0, maxChars)
    .map((char) => (isControl(char) ? ' ' : char))
    .join('')
  return chars.length > maxChars ? `${line}...` : line
}

function utf8Length(char: string): number {
  const code = char.codePointAt(0) ?? 0
  return code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4
}

/** `text` cut to at most `maxBytes` UTF-8 bytes at a character, with an ellipsis. */
function bounded(text: string, maxBytes: number): string {
  let total = 0
  for (const char of text) {
    total += utf8Length(char)
  }
  if (total <= maxBytes) {
    return text
  }
  const end = Math.max(0, maxBytes - 3)
  let used = 0
  let out = ''
  for (const char of text) {
    const size = utf8Length(char)
    if (used + size > end) {
      break
    }
    out += char
    used += size
  }
  return `${out}...`
}

/**
 * A validation failure naming each field and its messages, as the MCP
 * endpoint words one: at most twenty fields, each on one line, the whole
 * text cut to `maxBytes`.
 */
export function validationText(errors: Record<string, unknown>, maxBytes: number): string {
  const entries = Object.entries(errors)
  let text = 'The input is not valid:'
  for (const [field, messages] of entries.slice(0, MAX_REPORTED_FIELDS)) {
    const list = Array.isArray(messages)
      ? messages.map((message) => (typeof message === 'string' ? message : JSON.stringify(message)))
      : [typeof messages === 'string' ? messages : JSON.stringify(messages)]
    text += `\n- ${oneLine(field, MAX_FIELD_CHARS)}: ${oneLine(list.join(' '), MAX_MESSAGE_CHARS)}`
  }
  if (entries.length > MAX_REPORTED_FIELDS) {
    text += `\n(${entries.length - MAX_REPORTED_FIELDS} more fields are not valid.)`
  }
  return bounded(text, maxBytes)
}

/** Whether a content type is JSON: `application/json` or a `+json` type. */
function isJson(contentType: string | null): boolean {
  const media = (contentType ?? '').split(';')[0].trim().toLowerCase()
  return media === 'application/json' || media.endsWith('+json')
}

/** The `message` of an error body, else `fallback`. */
function messageOf(body: { value: unknown } | null, fallback: string): string {
  const message = body !== null && plainObject(body.value) ? body.value.message : undefined
  return typeof message === 'string' && message !== '' ? message : fallback
}

/**
 * The browser result of a route's answer. A JSON body appears only as
 * structured content. A failure becomes a result with `isError: true`
 * whose text the agent can act on.
 * A server failure shows nothing of its body.
 */
export function answerResult(
  status: number,
  contentType: string | null,
  body: string,
  maxBytes: number,
): ToolResult {
  let json: { value: unknown } | null = null
  if (isJson(contentType)) {
    try {
      json = { value: JSON.parse(body) as unknown }
    } catch {
      json = null
    }
  }
  if (status >= 200 && status <= 299) {
    if (body === '') {
      return { isError: false }
    }
    return json === null
      ? errorResult('The tool answered with a body that is not JSON.')
      : { structuredContent: json.value, isError: false }
  }
  if (status >= 300 && status <= 399) {
    return errorResult('The tool answered with a redirect, which a tool call cannot follow.')
  }
  if (status === 422) {
    const errors = json !== null && plainObject(json.value) ? json.value.errors : undefined
    return plainObject(errors)
      ? errorResult(validationText(errors, maxBytes))
      : errorResult(messageOf(json, 'The input is not valid.'))
  }
  if (status === 419) {
    return errorResult("The page's session expired: reload the page, then call the tool again.")
  }
  if (status === 429) {
    return errorResult('Too many calls: wait before calling the tool again.')
  }
  if (status >= 500) {
    return errorResult('The tool failed.')
  }
  const fallback =
    status === 401
      ? 'The tool needs a signed-in user.'
      : status === 403
        ? 'This action is unauthorized.'
        : status === 404
          ? 'Not found.'
          : 'The tool refused the call.'
  return errorResult(messageOf(json, fallback))
}

// ---------------------------------------------------------------------------
// Server tools (AGENT-014)
// ---------------------------------------------------------------------------

/** One piece of a route path, as the server splits it. */
export type PathPiece = { text: string } | { param: string; optional: boolean; catchAll: boolean }

/** A server tool a page lists. */
export type PageTool = {
  name: string
  title?: string
  description: string
  inputSchema: ToolInputSchema
  annotations: ToolAnnotations
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  path: PathPiece[]
}

/** The bounds of a server tool call, from the router's agents config. */
export type PageToolLimits = { maxResultBytes: number; callTimeoutMs: number }

/** The page's server tools: the Inertia page's `agentTools` field. */
export type PageTools = { limits: PageToolLimits; tools: PageTool[] }

const METHODS = new Set(['GET', 'POST', 'PUT', 'PATCH', 'DELETE'])
const HINTS = ['consequentialHint', 'debugging', 'readOnlyHint', 'untrustedContentHint']

/**
 * Whether `value` has every key in `required`. Keys a reader does not know
 * are ignored, so a later release can add one without breaking an app
 * scaffolded earlier; a change an older reader must not ignore bumps `v`.
 */
function hasKeys(value: Record<string, unknown>, required: string[]): boolean {
  return required.every((key) => Object.prototype.hasOwnProperty.call(value, key))
}

/** A hint as the page sends it: `false` when missing, `null` when not a boolean. */
function hint(annotations: Record<string, unknown>, key: string): boolean | null {
  const value = annotations[key]
  if (value === undefined) {
    return false
  }
  return typeof value === 'boolean' ? value : null
}

function positiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
}

function parsePiece(piece: unknown): PathPiece | null {
  if (!plainObject(piece)) {
    return null
  }
  const text = hasKeys(piece, ['text'])
  if (text === hasKeys(piece, ['param'])) {
    return null
  }
  if (text) {
    return typeof piece.text === 'string' ? { text: piece.text } : null
  }
  if (
    hasKeys(piece, ['param', 'optional', 'catchAll']) &&
    typeof piece.param === 'string' &&
    piece.param !== '' &&
    typeof piece.optional === 'boolean' &&
    typeof piece.catchAll === 'boolean'
  ) {
    return { param: piece.param, optional: piece.optional, catchAll: piece.catchAll }
  }
  return null
}

function parseTool(tool: unknown, names: Set<string>): PageTool | null {
  if (!plainObject(tool) || !hasKeys(tool, ['name', 'description', 'inputSchema', 'method', 'path'])) {
    return null
  }
  const { name, title, description, inputSchema, method, path } = tool
  const annotations = tool.annotations === undefined ? {} : tool.annotations
  if (
    typeof name !== 'string' ||
    !TOOL_NAME.test(name) ||
    names.has(name) ||
    (title !== undefined && (typeof title !== 'string' || title.trim() === '')) ||
    typeof description !== 'string' ||
    description.trim() === '' ||
    !plainObject(inputSchema) ||
    inputSchema.type !== 'object' ||
    !plainObject(annotations) ||
    !HINTS.every((key) => hint(annotations, key) !== null) ||
    typeof method !== 'string' ||
    !METHODS.has(method) ||
    !Array.isArray(path) ||
    path.length === 0
  ) {
    return null
  }
  const pieces = path.map(parsePiece)
  const first = pieces[0]
  if (pieces.some((piece) => piece === null) || first === null || !('text' in first) || !first.text.startsWith('/')) {
    return null
  }
  names.add(name)
  return {
    name,
    ...(typeof title === 'string' ? { title } : {}),
    description,
    inputSchema: inputSchema as ToolInputSchema,
    annotations: {
      readOnlyHint: hint(annotations, 'readOnlyHint') === true,
      untrustedContentHint: hint(annotations, 'untrustedContentHint') === true,
      consequentialHint: hint(annotations, 'consequentialHint') === true,
      debugging: hint(annotations, 'debugging') === true,
    },
    method: method as PageTool['method'],
    path: pieces as PathPiece[],
  }
}

/**
 * The page's server tools. A key this reader does not know is ignored at
 * every level, so a later release can add one without breaking an app
 * scaffolded earlier, and it never reaches a registration; a missing hint
 * reads as `false`. A change an older reader must not ignore bumps `v`, so
 * any `v` but 1 is refused, as are a known key of the wrong type, a missing
 * required key, a name outside the framework's grammar or used twice, and
 * more than {@link MAX_PAGE_TOOLS} tools. A refused payload is `null`, and
 * the page registers none of its tools rather than a guess.
 */
export function parsePageTools(value: unknown): PageTools | null {
  if (!plainObject(value) || !hasKeys(value, ['v', 'limits', 'tools']) || value.v !== 1) {
    return null
  }
  const { limits, tools } = value
  if (
    !plainObject(limits) ||
    !positiveInteger(limits.maxResultBytes) ||
    !positiveInteger(limits.callTimeoutMs) ||
    !Array.isArray(tools) ||
    tools.length > MAX_PAGE_TOOLS
  ) {
    return null
  }
  const names = new Set<string>()
  const parsed = tools.map((tool) => parseTool(tool, names))
  if (parsed.some((tool) => tool === null)) {
    return null
  }
  return {
    limits: { maxResultBytes: limits.maxResultBytes, callTimeoutMs: limits.callTimeoutMs },
    tools: parsed as PageTool[],
  }
}

/** A path, query argument or body value as text, or why it cannot be one. */
function scalarText(value: unknown): string | null | undefined {
  if (typeof value === 'string') {
    return value
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  return value === null || value === undefined ? null : undefined
}

/**
 * Whether a browser reads `segment` as a step of the path itself. The URL
 * parser treats `.` and `..`, and their percent-encoded forms, as dot
 * segments and resolves them before the request leaves, so such a value
 * would reach another route; a call with one is refused instead.
 */
function isDotSegment(segment: string): boolean {
  return segment === '.' || segment === '..'
}

/** `segment` percent-encoded as one path segment: all but unreserved characters. */
function encodeSegment(segment: string): string {
  if (segment !== '' && /^\.+$/u.test(segment)) {
    // Three dots or more: encoded as the server encodes them.
    return '%2E'.repeat(segment.length)
  }
  return encodeURIComponent(segment).replace(
    /[!'()*]/gu,
    (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
  )
}

/**
 * The path of a call: each parameter filled from the argument of its name
 * and percent-encoded, a catch-all encoded part by part, and an absent
 * optional parameter dropped with the `/` before it. `filled` names the
 * parameters that filled a segment.
 */
export function fillPath(
  path: PathPiece[],
  args: Record<string, unknown>,
): { path: string; filled: string[] } | { error: string } {
  let out = ''
  const filled: string[] = []
  let previous: PathPiece | null = null
  for (const piece of path) {
    if ('text' in piece) {
      out += piece.text
      previous = piece
      continue
    }
    const text = scalarText(args[piece.param])
    if (text === undefined) {
      return { error: `the path parameter \`${piece.param}\` must be a string, a number or a boolean` }
    }
    if (text !== null && text !== '') {
      const parts = piece.catchAll ? text.split('/') : [text]
      if (parts.some(isDotSegment)) {
        return {
          error: `the path parameter \`${piece.param}\` cannot be \`.\` or \`..\`, or hold one as a part, because a browser resolves it as a step of the path`,
        }
      }
      try {
        out += piece.catchAll ? text.split('/').map(encodeSegment).join('/') : encodeSegment(text)
      } catch {
        return { error: `the path parameter \`${piece.param}\` is not valid text` }
      }
      filled.push(piece.param)
    } else if (piece.optional) {
      if (previous !== null && 'text' in previous && previous.text.endsWith('/')) {
        out = out.slice(0, -1)
      }
    } else {
      return { error: `the path parameter \`${piece.param}\` is missing` }
    }
    previous = piece
  }
  return { path: out === '' ? '/' : out, filled }
}

/** The request a call of `tool` with `args` sends, or why it cannot be sent. */
export function toolRequest(
  tool: PageTool,
  args: Record<string, unknown>,
): { path: string; body?: string } | { error: string } {
  const filled = fillPath(tool.path, args)
  if ('error' in filled) {
    return filled
  }
  if (tool.method !== 'GET') {
    return { path: filled.path, body: JSON.stringify(args) }
  }
  // A browser cannot send a GET body, so a GET tool's arguments go in the
  // query, in argument order: every scalar one, path arguments included,
  // since a form request on GET reads its query. A null, and a path
  // argument that filled no segment, stay out.
  const inPath = new Set(tool.path.flatMap((piece) => ('param' in piece ? [piece.param] : [])))
  const query = new URLSearchParams()
  for (const [name, value] of Object.entries(args)) {
    if (inPath.has(name) && !filled.filled.includes(name)) {
      continue
    }
    const text = scalarText(value)
    if (text === undefined) {
      return {
        error: `the argument \`${name}\` must be a string, a number or a boolean, because a GET tool sends its arguments in the query`,
      }
    }
    if (text !== null) {
      query.append(name, text)
    }
  }
  const search = query.toString()
  return { path: search === '' ? filled.path : `${filled.path}?${search}` }
}

/** The body, or `null` when it is larger than `maxBytes`. */
async function readBounded(response: Response, maxBytes: number): Promise<string | null> {
  if (response.body === null) {
    return ''
  }
  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let total = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) {
      break
    }
    total += value.byteLength
    if (total > maxBytes) {
      void reader.cancel().catch(() => undefined)
      return null
    }
    chunks.push(value)
  }
  const bytes = new Uint8Array(total)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return new TextDecoder().decode(bytes)
}

/** A finished call: its result, and whether the server answered it. */
export type PageToolCall = { result: ToolResult; answered: boolean }

/**
 * Call the server tool `tool`: its route on this origin, with the session
 * cookie, the CSRF header and the tool-call header, the arguments in the
 * body (or the query of a GET), bounded by `limits`. A redirect is never
 * followed, so no header reaches another origin. Rejects with an
 * `AbortError` when the agent aborts; every other failure is a result.
 */
export async function callPageTool(
  tool: PageTool,
  input: unknown,
  limits: PageToolLimits,
  client?: ToolClient,
): Promise<PageToolCall> {
  const signal = client?.signal
  if (signal?.aborted === true) {
    throw abortError()
  }
  if (!plainObject(input)) {
    return { result: invalidInput('the arguments must be an object'), answered: false }
  }
  const request = toolRequest(tool, input)
  if ('error' in request) {
    return { result: invalidInput(request.error), answered: false }
  }
  // Built from the page's own origin and the server's path pieces, and
  // checked before any credential is attached.
  const url = new URL(`${window.location.origin}${request.path}`)
  if (url.origin !== window.location.origin) {
    return { result: invalidInput("the tool's path leaves the page's origin"), answered: false }
  }
  const headers: Record<string, string> = {
    Accept: 'application/json',
    [TOOL_CALL_HEADER]: tool.name,
    ...csrfHeaders(),
  }
  if (request.body !== undefined) {
    headers['Content-Type'] = 'application/json'
  }
  const controller = new AbortController()
  let timedOut = false
  const timer = setTimeout(() => {
    timedOut = true
    controller.abort()
  }, limits.callTimeoutMs)
  const onAbort = () => controller.abort()
  signal?.addEventListener('abort', onAbort)
  const unanswered = (answered: boolean): PageToolCall => {
    if (aborted(signal)) {
      throw abortError()
    }
    return {
      result: timedOut
        ? errorResult(
            `The tool did not finish within ${limits.callTimeoutMs / 1000} seconds and was stopped.`,
          )
        : errorResult('The tool could not be reached.'),
      answered,
    }
  }
  try {
    let response: Response
    try {
      response = await fetch(url.href, {
        method: tool.method,
        headers,
        ...(request.body === undefined ? {} : { body: request.body }),
        credentials: 'same-origin',
        redirect: 'manual',
        cache: 'no-store',
        signal: controller.signal,
      })
    } catch {
      return unanswered(false)
    }
    if (response.type === 'opaqueredirect') {
      return {
        result: errorResult('The tool answered with a redirect, which a tool call cannot follow.'),
        answered: true,
      }
    }
    let body: string | null
    try {
      body = await readBounded(response, limits.maxResultBytes)
    } catch {
      return unanswered(true)
    }
    if (body === null) {
      return {
        result: errorResult(
          `The tool's result is larger than the ${limits.maxResultBytes} bytes the page returns.`,
        ),
        answered: true,
      }
    }
    return {
      result: answerResult(
        response.status,
        response.headers.get('Content-Type'),
        body,
        limits.maxResultBytes,
      ),
      answered: true,
    }
  } finally {
    clearTimeout(timer)
    signal?.removeEventListener('abort', onAbort)
  }
}

// ---------------------------------------------------------------------------
// Component tools and form tools (AGENT-015)
// ---------------------------------------------------------------------------

/** A tool a component registers for as long as it is mounted. */
export type PageToolOptions<TInput = Record<string, unknown>> = {
  /** Unique on the page, in the framework's tool name grammar. */
  name: string
  title?: string
  /** What the agent reads to decide on a call; write it as a code literal. */
  description: string
  inputSchema: ToolInputSchema
  readOnly?: boolean
  untrustedContent?: boolean
  consequential?: boolean
  /** Meant for debugging and developer tooling, not end users. */
  debugging?: boolean
  /**
   * Answers a call: a {@link ToolResult}, or any JSON value, which becomes
   * the result's structured content. A string becomes text. A thrown
   * error becomes a tool error with its message.
   */
  execute: (input: TInput, signal?: AbortSignal) => unknown
}

function isToolResult(value: unknown): value is ToolResult {
  return plainObject(value) && typeof value.isError === 'boolean'
    && (Array.isArray(value.content)
      || Object.prototype.hasOwnProperty.call(value, 'structuredContent')
      || Object.keys(value).length === 1)
}

/** `value` as a tool result. */
function resultOf(value: unknown): ToolResult {
  if (isToolResult(value)) {
    return Object.prototype.hasOwnProperty.call(value, 'structuredContent')
      ? { structuredContent: value.structuredContent, isError: value.isError }
      : value
  }
  if (value === undefined) {
    return { isError: false }
  }
  if (typeof value === 'string') {
    return { content: [{ type: 'text', text: value }], isError: false }
  }
  return { structuredContent: value, isError: false }
}

/**
 * The definition of a component's tool. `execute` is read at each call, so
 * a component can hand a new closure every render without registering
 * again.
 */
export function componentTool<TInput>(
  options: PageToolOptions<TInput>,
  execute: () => PageToolOptions<TInput>['execute'],
): ToolDefinition {
  return {
    name: options.name,
    ...(options.title === undefined ? {} : { title: options.title }),
    description: options.description,
    inputSchema: options.inputSchema,
    annotations: {
      readOnlyHint: options.readOnly === true,
      untrustedContentHint: options.untrustedContent === true,
      consequentialHint: options.consequential === true,
      debugging: options.debugging === true,
    },
    execute: async (input, client) => {
      const signal = client?.signal
      if (aborted(signal)) {
        throw abortError()
      }
      if (!plainObject(input)) {
        return invalidInput('the arguments must be an object')
      }
      try {
        return resultOf(await execute()(input as TInput, signal))
      } catch (error) {
        if (aborted(signal) || (error instanceof DOMException && error.name === 'AbortError')) {
          throw error
        }
        const message = oneLine(error instanceof Error ? error.message : String(error), MAX_MESSAGE_CHARS)
        return errorResult(message === '' ? 'The tool failed.' : message)
      }
    },
  }
}

/** An Inertia visit method. */
export type FormMethod = 'get' | 'post' | 'put' | 'patch' | 'delete'

/** Where a form submits: a generated route's `{ url, method }`, or your own. */
export type FormAction = { url: string; method: string }

/** A form tool. */
export type FormToolOptions = {
  /** Unique on the page, in the framework's tool name grammar. */
  name: string
  title?: string
  /** What the agent reads to decide on a call; write it as a code literal. */
  description: string
  /**
   * The form's server request type's schema, from `suprnova
   * generate-types`: `toolSchemas.StoreNoteRequest` in
   * `src/types/tool-schemas.ts`.
   */
  schema: ToolInputSchema
  action: FormAction
  /**
   * Fill the form and leave the submit to the user. Always so for a
   * `DELETE` action, whatever this says.
   */
  destructive?: boolean
  /** Meant for debugging and developer tooling, not end users. */
  debugging?: boolean
}

/** The options of the Inertia visit a form tool submits with. */
export type FormVisit = {
  preserveScroll: boolean
  /** The form-tool header, which a consent page refuses. */
  headers: Record<string, string>
  onSuccess: (page: { component: string; url: string }) => void
  onError: (errors: Record<string, unknown>) => void
  onFinish: () => void
}

/** What a form tool needs of the kit's form: fill it, submit it, cancel it. */
export type FormDriver = {
  fill: (values: Record<string, unknown>) => void
  submit: (method: FormMethod, url: string, visit: FormVisit) => void
  cancel: () => void
}

function formMethod(method: string): FormMethod {
  const lower = method.toLowerCase()
  return lower === 'get' || lower === 'put' || lower === 'patch' || lower === 'delete' ? lower : 'post'
}

function list(names: string[]): string {
  return names.map((name) => `\`${name}\``).join(', ')
}

/**
 * The definition of a form tool. A call fills the form, which the user
 * sees, and submits it through Inertia as the user's own click would: the
 * route's form request validates it, and its validation errors come back
 * as a tool error naming each field. A destructive form is filled and left
 * for the user to submit.
 *
 * A form tool sends no tool-call header: the form is the page's own, sent
 * through the ordinary Inertia visit, so the route's own throttle and
 * checks apply and the server tool budget does not. It marks the visit
 * with the form-tool header instead, which a consent page refuses, so a
 * form tool on the approval or claim page cannot approve or claim.
 */
export function formTool(options: FormToolOptions, driver: () => FormDriver): ToolDefinition {
  const method = formMethod(options.action.method)
  const destructive = options.destructive === true || method === 'delete'
  const schema = options.schema
  const properties = plainObject(schema.properties) ? Object.keys(schema.properties) : []
  return {
    name: options.name,
    ...(options.title === undefined ? {} : { title: options.title }),
    description: options.description,
    inputSchema: schema,
    annotations: {
      readOnlyHint: method === 'get',
      untrustedContentHint: false,
      consequentialHint: destructive,
      debugging: options.debugging === true,
    },
    execute: async (input, client) => {
      const signal = client?.signal
      if (aborted(signal)) {
        throw abortError()
      }
      if (!plainObject(input)) {
        return invalidInput('the arguments must be an object')
      }
      const unknown = Object.keys(input).filter((key) => !properties.includes(key))
      if (schema.additionalProperties === false && unknown.length > 0) {
        return invalidInput(`the form has no field ${list(unknown)}`)
      }
      const values: Record<string, unknown> = {}
      for (const key of properties) {
        if (Object.prototype.hasOwnProperty.call(input, key)) {
          values[key] = input[key]
        }
      }
      const form = driver()
      form.fill(values)
      if (destructive) {
        return {
          structuredContent: { outcome: 'filled', submitted: false },
          isError: false,
        }
      }
      return new Promise<ToolResult>((resolve, reject) => {
        let settled = false
        const settle = (result: ToolResult) => {
          if (!settled) {
            settled = true
            signal?.removeEventListener('abort', onAbort)
            resolve(result)
          }
        }
        const onAbort = () => {
          if (!settled) {
            settled = true
            form.cancel()
            reject(abortError())
          }
        }
        signal?.addEventListener('abort', onAbort)
        try {
          form.submit(method, options.action.url, {
            preserveScroll: true,
            headers: { [FORM_TOOL_HEADER]: '1' },
            onSuccess: (page) => {
              const outcome = { outcome: 'submitted', component: page.component, url: page.url }
              settle({
                structuredContent: outcome,
                isError: false,
              })
            },
            onError: (errors) => {
              settle({
                structuredContent: { outcome: 'invalid', errors },
                isError: true,
              })
            },
            onFinish: () => {
              settle(
                errorResult(
                  'The form was sent, but the visit ended without an answer: read the page before calling again.',
                ),
              )
            },
          })
        } catch (error) {
          settle(errorResult(`The form could not be submitted: ${oneLine(String(error), MAX_MESSAGE_CHARS)}`))
        }
      })
    },
  }
}
