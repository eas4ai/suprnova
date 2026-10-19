// The server tools of each Inertia page, registered with WebMCP while the
// page is shown (AGENT-014).
//
// The server decides which tools a page offers each viewer and sends them
// as the page object's `agentTools` field, on every response, partial
// reloads included. `installPageTools` follows Inertia's `navigate` event,
// which fires for the first page, every visit and every history step, and
// keeps the registered tools equal to the current page's: a tool that left
// is removed, a new one registered, and one whose definition changed
// registered again. After a call of a tool not marked read-only, the page
// reloads every prop before the call resolves, so what the agent reads
// next is what it did.

import {
  DEFAULT_MAX_RESULT_BYTES,
  callPageTool,
  parsePageTools,
  registerTool,
  type PageTool,
  type PageToolLimits,
  type ToolDefinition,
} from './core'

/** The part of an Inertia page this reads. */
export type InertiaPageLike = { component: string; agentTools?: unknown }

/** The part of Inertia's `router` this uses. */
export interface InertiaRouterLike {
  on(type: 'navigate', callback: (event: CustomEvent<{ page: InertiaPageLike }>) => void): () => void
  reload(options?: { onFinish?: () => void }): void
}

const DEFAULT_LIMITS: PageToolLimits = {
  maxResultBytes: DEFAULT_MAX_RESULT_BYTES,
  callTimeoutMs: 30_000,
}

/**
 * Reload every prop of the current page, as `router.reload()` does with no
 * `only`, and resolve when the visit finishes or `timeoutMs` passes. A
 * reload that fails leaves the tool's result as it was.
 */
function refresh(router: InertiaRouterLike, timeoutMs: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, timeoutMs)
    const done = () => {
      clearTimeout(timer)
      resolve()
    }
    try {
      router.reload({ onFinish: done })
    } catch {
      done()
    }
  })
}

/**
 * Register each Inertia page's server tools for as long as the page is
 * shown, and return the function that stops. Call it before
 * `createInertiaApp`, so the listener hears the first page's `navigate`.
 * Outside the browser it does nothing.
 */
export function installPageTools(router: InertiaRouterLike): () => void {
  if (typeof window === 'undefined') {
    return () => undefined
  }
  const registered = new Map<string, { signature: string; leave: () => void }>()
  let limits = DEFAULT_LIMITS

  const definition = (tool: PageTool): ToolDefinition => ({
    name: tool.name,
    ...(tool.title === undefined ? {} : { title: tool.title }),
    description: tool.description,
    inputSchema: tool.inputSchema,
    annotations: tool.annotations,
    execute: async (input, client) => {
      const current = limits
      const call = await callPageTool(tool, input, current, client)
      // Only the server knows whether a refused or failed call changed
      // anything, so any answer refreshes; no answer means nothing ran.
      if (!tool.annotations.readOnlyHint && call.answered) {
        await refresh(router, current.callTimeoutMs)
      }
      return call.result
    },
  })

  const show = (page: InertiaPageLike) => {
    let tools: PageTool[] = []
    if (page.agentTools !== undefined) {
      const parsed = parsePageTools(page.agentTools)
      if (parsed === null) {
        console.error('The page lists its agent tools in a shape this page cannot read; none are registered.')
      } else {
        limits = parsed.limits
        tools = parsed.tools
      }
    }
    const next = new Map(tools.map((tool) => [tool.name, tool]))
    for (const [name, entry] of registered) {
      const tool = next.get(name)
      if (tool === undefined || JSON.stringify(tool) !== entry.signature) {
        entry.leave()
        registered.delete(name)
      }
    }
    for (const tool of tools) {
      if (!registered.has(tool.name)) {
        registered.set(tool.name, {
          signature: JSON.stringify(tool),
          leave: registerTool(definition(tool)),
        })
      }
    }
  }

  const stop = router.on('navigate', (event) => show(event.detail.page))
  return () => {
    stop()
    for (const entry of registered.values()) {
      entry.leave()
    }
    registered.clear()
  }
}
