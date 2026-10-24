// Stand-ins for a kit's agent tool tests in jsdom: WebMCP's
// `document.modelContext`, Inertia's router events, and `fetch`.

import type { ToolClient, ToolDefinition, ToolResult } from './src/lib/agents/core'

/** One live registration: the descriptor the page passed and its signal. */
type Live = { tool: ToolDefinition; signal?: AbortSignal }

/**
 * A model context that keeps the tools registered with it, refuses a
 * duplicate name as Chromium does, and drops a tool when its signal aborts.
 */
export class FakeModelContext {
  readonly live = new Map<string, Live>()
  /** Every descriptor ever passed, in order, for checking what the page sent. */
  readonly descriptors: ToolDefinition[] = []

  registerTool(tool: ToolDefinition, options?: { signal?: AbortSignal }): Promise<void> {
    if (this.live.has(tool.name)) {
      return Promise.reject(new Error(`duplicate tool ${tool.name}`))
    }
    this.descriptors.push(tool)
    const entry = { tool, signal: options?.signal }
    this.live.set(tool.name, entry)
    options?.signal?.addEventListener('abort', () => {
      if (this.live.get(tool.name) === entry) {
        this.live.delete(tool.name)
      }
    })
    return Promise.resolve()
  }

  /** The registered tool names, sorted. */
  names(): string[] {
    return [...this.live.keys()].sort()
  }

  /** Call a registered tool as an agent would. */
  call(name: string, input: unknown, client?: ToolClient): Promise<ToolResult> {
    const entry = this.live.get(name)
    if (entry === undefined) {
      throw new Error(`no tool ${name}; registered: ${this.names().join(', ')}`)
    }
    return entry.tool.execute(input, client)
  }
}

/** Put a fresh fake model context on the document. */
export function installModelContext(): FakeModelContext {
  const context = new FakeModelContext()
  Object.defineProperty(document, 'modelContext', { value: context, configurable: true })
  return context
}

/** Take the model context off the document again. */
export function removeModelContext(): void {
  Reflect.deleteProperty(document, 'modelContext')
}

/** Inertia's router as `installPageTools` sees it, driven by the test. */
export function fakeRouter() {
  const listeners = new Set<(event: CustomEvent<{ page: { component: string; agentTools?: unknown } }>) => void>()
  const reloads: Array<{ onFinish?: () => void }> = []
  return {
    reloads,
    on(
      _type: 'navigate',
      callback: (event: CustomEvent<{ page: { component: string; agentTools?: unknown } }>) => void,
    ) {
      listeners.add(callback)
      return () => {
        listeners.delete(callback)
      }
    },
    reload(options?: { onFinish?: () => void }) {
      reloads.push(options ?? {})
    },
    /** Fire `navigate` for `page`, as Inertia does on every page it shows. */
    navigate(page: { component: string; agentTools?: unknown }) {
      for (const listener of [...listeners]) {
        listener(new CustomEvent('inertia:navigate', { detail: { page } }))
      }
    },
  }
}

/** One request the fake `fetch` received. */
export type Sent = { url: string; init: RequestInit }

/** A `fetch` that records each request and answers with `answer`. */
export function fakeFetch(answer: (sent: Sent) => Promise<Response> | Response) {
  const sent: Sent[] = []
  const fetch = (url: string, init: RequestInit = {}) => {
    const request = { url, init }
    sent.push(request)
    return Promise.resolve(answer(request))
  }
  return { fetch, sent }
}

/** A JSON answer. */
export function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

/** Lets pending promises and the timers they queue run. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}
