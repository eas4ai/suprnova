// The kit-agnostic half of the agent tools, run in each kit: the shared
// browser-call cases every implementation reads (AGENT-014), the page
// tools `installPageTools` registers from Inertia's `navigate` events, and
// the registry every tool goes through (AGENT-015).

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import cases from './browser-calls.json'
import {
  RESULT_HOLD_MS,
  TOOL_CALL_HEADER,
  callPageTool,
  registerTool,
  type PageTool,
  type ToolDefinition,
} from './src/lib/agents/core'
import { installPageTools } from './src/lib/agents/inertia'
import { fakeFetch, fakeRouter, installModelContext, json, removeModelContext, settle, type FakeModelContext } from './fakes'

type PathCase = (typeof cases.paths)[number]
type CsrfCase = (typeof cases.csrf)[number]
type ResultCase = (typeof cases.results)[number]

const LIMITS = { maxResultBytes: 8_388_608, callTimeoutMs: 30_000 }
const SCHEMA = { type: 'object' as const, properties: {} }
const HINTS = { readOnlyHint: false, untrustedContentHint: false, consequentialHint: false }

function serverTool(overrides: Partial<PageTool> = {}): PageTool {
  return {
    name: 'fixture.tool',
    description: 'A fixture tool.',
    inputSchema: SCHEMA,
    annotations: HINTS,
    method: 'POST',
    path: [{ text: '/fixture' }],
    ...overrides,
  }
}

let context: FakeModelContext

beforeEach(() => {
  context = installModelContext()
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  removeModelContext()
  document.head.innerHTML = ''
})

describe('agent_014_server_tool_paths_match_the_shared_cases', () => {
  test.each(cases.paths.map((entry: PathCase) => [entry.name, entry] as const))('%s', async (_name, entry) => {
    const server = fakeFetch(() => json({ ok: true }))
    vi.stubGlobal('fetch', server.fetch)
    const tool = serverTool({ method: entry.method as PageTool['method'], path: entry.path as PageTool['path'] })
    const call = await callPageTool(tool, entry.arguments, LIMITS)
    const expected = entry.expected as { url?: string; body?: unknown; error?: unknown }
    if (expected.error !== undefined) {
      expect(call.result).toEqual(expected.error)
      expect(server.sent).toHaveLength(0)
      return
    }
    const wanted = `${window.location.origin}${expected.url}`
    if (new URL(wanted).href !== wanted) {
      // The browser's URL parser resolves a `.` or `..` segment, encoded or
      // not, before a request leaves, so this URL cannot be sent as the case
      // gives it: the call would reach another route. The page refuses it.
      expect(call.result.isError).toBe(true)
      expect(call.result.content[0]?.text).toMatch(/^Invalid input: the path parameter `\w+` cannot be `\.` or `\.\.`/u)
      expect(server.sent).toHaveLength(0)
      return
    }
    expect(server.sent).toHaveLength(1)
    const [request] = server.sent
    expect(request.url).toBe(wanted)
    expect(request.init.method).toBe(entry.method)
    if (expected.body === null) {
      expect(request.init.body).toBeUndefined()
    } else {
      expect(JSON.parse(String(request.init.body))).toEqual(expected.body)
    }
  })
})

describe('agent_014_server_tool_csrf_headers_match_the_shared_cases', () => {
  test.each(cases.csrf.map((entry: CsrfCase) => [entry.name, entry] as const))('%s', async (_name, entry) => {
    if (entry.meta !== null) {
      const meta = document.createElement('meta')
      meta.name = 'csrf-token'
      meta.content = entry.meta
      document.head.append(meta)
    }
    vi.spyOn(document, 'cookie', 'get').mockReturnValue(entry.cookie ?? '')
    const server = fakeFetch(() => json({ ok: true }))
    vi.stubGlobal('fetch', server.fetch)
    await callPageTool(serverTool(), {}, LIMITS)
    const headers = server.sent[0].init.headers as Record<string, string>
    const csrf = Object.fromEntries(
      Object.entries(headers).filter(([name]) => name === 'X-CSRF-TOKEN' || name === 'X-XSRF-TOKEN'),
    )
    expect(csrf).toEqual(entry.expected)
  })
})

/** A response as the case gives it, or the failure it names. */
function answer(response: ResultCase['response'], signal: AbortSignal | null | undefined): Promise<Response> {
  const given = response as {
    failure?: string
    status?: number
    type?: string
    contentType?: string | null
    body?: string
  }
  if (given.failure === 'network') {
    return Promise.reject(new TypeError('Failed to fetch'))
  }
  if (given.failure === 'timeout') {
    return new Promise((_resolve, reject) => {
      signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
    })
  }
  if (given.type === 'opaqueredirect') {
    return Promise.resolve({ type: 'opaqueredirect', status: 0, headers: new Headers(), body: null } as unknown as Response)
  }
  const headers = given.contentType === null || given.contentType === undefined ? {} : { 'Content-Type': given.contentType }
  return Promise.resolve(new Response(given.body === '' ? null : given.body, { status: given.status, headers }))
}

describe('agent_014_server_tool_results_match_the_shared_cases', () => {
  test.each(cases.results.map((entry: ResultCase) => [entry.name, entry] as const))('%s', async (_name, entry) => {
    const timeout = (entry.response as { callTimeoutMs?: number }).callTimeoutMs
    if (timeout !== undefined) {
      vi.useFakeTimers()
    }
    vi.stubGlobal('fetch', (_url: string, init: RequestInit) => answer(entry.response, init.signal))
    const pending = callPageTool(serverTool(), {}, {
      maxResultBytes: entry.maxResultBytes,
      callTimeoutMs: timeout ?? 30_000,
    })
    if (timeout !== undefined) {
      await vi.advanceTimersByTimeAsync(timeout)
    }
    const call = await pending
    expect(call.result).toEqual(entry.expected)
  })
})

/** A page whose `agentTools` lists `tools`. */
function page(component: string, tools: PageTool[]) {
  return { component, agentTools: { v: 1, limits: LIMITS, tools } }
}

const search = serverTool({
  name: 'posts.search',
  description: 'Search the posts.',
  method: 'GET',
  path: [{ text: '/posts' }],
  annotations: { readOnlyHint: true, untrustedContentHint: true, consequentialHint: false },
})
const create = serverTool({
  name: 'posts.create',
  title: 'Create a post',
  description: 'Create a draft post.',
  path: [{ text: '/posts' }],
})
const destroy = serverTool({
  name: 'posts.delete',
  description: 'Delete a post.',
  method: 'DELETE',
  path: [{ text: '/posts/' }, { param: 'post', optional: false, catchAll: false }],
  annotations: { readOnlyHint: false, untrustedContentHint: false, consequentialHint: true },
})
const help = serverTool({ name: 'help.search', description: 'Search the help.', path: [{ text: '/help' }] })

test('agent_014_page_tools_register_with_their_annotations', () => {
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [search, create, destroy]))
  expect(context.names()).toEqual(['posts.create', 'posts.delete', 'posts.search'])
  const descriptor = (name: string) => context.descriptors.find((tool) => tool.name === name) as ToolDefinition
  expect(descriptor('posts.search').annotations).toEqual({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
  })
  expect(descriptor('posts.delete').annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
  })
  expect(descriptor('posts.create').title).toBe('Create a post')
  expect(descriptor('posts.create').description).toBe('Create a draft post.')
  expect(descriptor('posts.create').inputSchema).toEqual(SCHEMA)
  stop()
  expect(context.names()).toEqual([])
})

test('agent_014_inertia_visit_replaces_the_page_tools', () => {
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [help, search, create]))
  const firstHelp = context.live.get('help.search')?.tool
  // The next page keeps the shared tool, drops the route's, adds its own
  // and changes the description of one it keeps.
  const changed = { ...create, description: 'Create a post as a draft.' }
  router.navigate(page('Posts/Show', [help, changed, destroy]))
  expect(context.names()).toEqual(['help.search', 'posts.create', 'posts.delete'])
  expect(context.live.get('help.search')?.tool).toBe(firstHelp)
  expect(context.live.get('posts.create')?.tool.description).toBe('Create a post as a draft.')
  // A page that lists none removes every one.
  router.navigate({ component: 'Home' })
  expect(context.names()).toEqual([])
  stop()
})

test('agent_014_history_back_restores_that_pages_tools', () => {
  const router = fakeRouter()
  const stop = installPageTools(router)
  const first = page('Posts/Index', [help, search])
  router.navigate(first)
  router.navigate(page('Posts/Show', [help, destroy]))
  // A history step hands back the earlier page object, `agentTools` and all.
  router.navigate(first)
  expect(context.names()).toEqual(['help.search', 'posts.search'])
  stop()
})

test('agent_014_invalid_page_tools_register_nothing', () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)
  const router = fakeRouter()
  const stop = installPageTools(router)
  const broken = { ...create, extra: true }
  router.navigate({ component: 'Posts/Index', agentTools: { v: 1, limits: LIMITS, tools: [search, broken] } })
  expect(context.names()).toEqual([])
  router.navigate({ component: 'Posts/Index', agentTools: { v: 2, limits: LIMITS, tools: [search] } })
  expect(context.names()).toEqual([])
  expect(error).toHaveBeenCalled()
  stop()
})

test('agent_014_server_tool_call_carries_session_csrf_and_header', async () => {
  document.head.innerHTML = '<meta name="csrf-token" content="page-token">'
  const server = fakeFetch(() => json({ id: 7 }, 201))
  vi.stubGlobal('fetch', server.fetch)
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [create]))
  const pending = context.call('posts.create', { title: 'Hello' })
  await settle()
  router.reloads[0]?.onFinish?.()
  const result = await pending
  expect(result).toEqual({
    content: [{ type: 'text', text: '{"id":7}' }],
    structuredContent: { id: 7 },
    isError: false,
  })
  const [request] = server.sent
  const headers = request.init.headers as Record<string, string>
  expect(request.url).toBe(`${window.location.origin}/posts`)
  expect(request.init.credentials).toBe('same-origin')
  expect(request.init.redirect).toBe('manual')
  expect(request.init.cache).toBe('no-store')
  expect(headers[TOOL_CALL_HEADER]).toBe('posts.create')
  expect(headers['X-CSRF-TOKEN']).toBe('page-token')
  expect(headers.Accept).toBe('application/json')
  expect(headers['Content-Type']).toBe('application/json')
  expect(JSON.parse(String(request.init.body))).toEqual({ title: 'Hello' })
  stop()
})

test('agent_014_write_tool_reloads_the_page_props', async () => {
  vi.stubGlobal('fetch', fakeFetch(() => json({ errors: { title: ['Required.'] } }, 422)).fetch)
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [create]))
  let resolved = false
  const pending = context.call('posts.create', {}).then((result) => {
    resolved = true
    return result
  })
  await settle()
  // A refused write still reloads: only the server knows what it changed.
  // The reload asks for every prop, and the call waits for it.
  expect(router.reloads).toHaveLength(1)
  expect(Object.keys(router.reloads[0])).toEqual(['onFinish'])
  expect(resolved).toBe(false)
  router.reloads[0].onFinish?.()
  const result = await pending
  expect(result.isError).toBe(true)
  stop()
})

test('agent_014_read_only_tool_does_not_reload', async () => {
  vi.stubGlobal('fetch', fakeFetch(() => json([])).fetch)
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [search]))
  const result = await context.call('posts.search', { q: 'x' })
  expect(result.isError).toBe(false)
  expect(router.reloads).toHaveLength(0)
  stop()
})

test('agent_014_unanswered_call_does_not_reload', async () => {
  vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')))
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [create]))
  const result = await context.call('posts.create', {})
  expect(result).toEqual({ content: [{ type: 'text', text: 'The tool could not be reached.' }], isError: true })
  expect(router.reloads).toHaveLength(0)
  stop()
})

test('agent_014_an_aborted_call_rejects_and_does_not_reload', async () => {
  vi.stubGlobal('fetch', (_url: string, init: RequestInit) =>
    new Promise((_resolve, reject) => {
      init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
    }),
  )
  const router = fakeRouter()
  const stop = installPageTools(router)
  router.navigate(page('Posts/Index', [create]))
  const controller = new AbortController()
  const pending = context.call('posts.create', {}, { signal: controller.signal })
  controller.abort()
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  expect(router.reloads).toHaveLength(0)
  stop()
})

function tool(name: string, execute: ToolDefinition['execute'] = async () => ({ content: [], isError: false })): ToolDefinition {
  return { name, description: `The ${name} tool.`, inputSchema: SCHEMA, annotations: HINTS, execute }
}

test('agent_015_duplicate_tool_name_is_refused', () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => undefined)
  const leave = registerTool(tool('notes.save'))
  const second = registerTool(tool('notes.save'))
  expect(context.names()).toEqual(['notes.save'])
  expect(error).toHaveBeenCalledWith(expect.stringContaining('another tool on the page has that name'))
  registerTool(tool('not a name'))
  expect(error).toHaveBeenCalledWith(expect.stringContaining('was not registered'))
  expect(context.names()).toEqual(['notes.save'])
  second()
  expect(context.names()).toEqual(['notes.save'])
  leave()
  expect(context.names()).toEqual([])
})

test('agent_015_leaving_tool_holds_until_its_pending_call_settles', async () => {
  vi.useFakeTimers()
  let finish: (result: { content: never[]; isError: boolean }) => void = () => undefined
  const leave = registerTool(
    tool('notes.save', () => new Promise((resolve) => {
      finish = resolve
    })),
  )
  const execute = context.live.get('notes.save')?.tool.execute
  const first = context.call('notes.save', {})
  leave()
  // Still registered while its call is pending; a new call is refused.
  expect(context.names()).toEqual(['notes.save'])
  const refused = await execute?.({})
  expect(refused?.isError).toBe(true)
  // A tool of the same name waits for the old one to go.
  const next = registerTool(tool('notes.save'))
  finish({ content: [], isError: false })
  await first
  await vi.advanceTimersByTimeAsync(RESULT_HOLD_MS - 1)
  expect(context.live.get('notes.save')?.tool).toBeDefined()
  expect(context.descriptors.filter((each) => each.name === 'notes.save')).toHaveLength(1)
  await vi.advanceTimersByTimeAsync(1)
  expect(context.descriptors.filter((each) => each.name === 'notes.save')).toHaveLength(2)
  expect(context.names()).toEqual(['notes.save'])
  next()
  expect(context.names()).toEqual([])
})

test('agent_015_back_forward_cache_suspends_and_restores_tools', () => {
  const leave = registerTool(tool('notes.save'))
  window.dispatchEvent(Object.assign(new Event('pagehide'), { persisted: true }))
  expect(context.names()).toEqual([])
  window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: true }))
  expect(context.names()).toEqual(['notes.save'])
  leave()
  expect(context.names()).toEqual([])
})

test('agent_015_a_page_without_webmcp_registers_nothing', () => {
  removeModelContext()
  const leave = registerTool(tool('notes.save'))
  leave()
  expect(Reflect.get(document, 'modelContext')).toBeUndefined()
})
