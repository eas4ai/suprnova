// The React kit's agent hooks in jsdom, against a fake model context and
// Inertia's own `useForm` with the router's visits recorded (AGENT-015).

import { router, useForm } from '@inertiajs/react'
import { act, createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { useFormTool, usePageTool } from './src/lib/agents'
import { toolSchemas } from './src/types/tool-schemas'
import { installModelContext, removeModelContext, type FakeModelContext } from './fakes'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const SCHEMA = { type: 'object' as const, properties: { amount: { type: 'integer' } } }

function probe<P extends object>(render: (props: P) => void, props: P) {
  const Probe = (current: P) => {
    render(current)
    return null
  }
  const root = createRoot(document.createElement('div'))
  act(() => root.render(createElement(Probe, props)))
  return {
    rerender: (next: P) => act(() => root.render(createElement(Probe, next))),
    unmount: () => act(() => root.unmount()),
  }
}

type Visit = { method: string; url: string; data: unknown }

/** Record every Inertia visit instead of sending it, answering with `answer`. */
function recordVisits(answer: (options: Record<string, (...args: never[]) => unknown>) => void): Visit[] {
  const visits: Visit[] = []
  for (const method of ['get', 'post', 'put', 'patch'] as const) {
    vi.spyOn(router, method).mockImplementation(((url: string, data: unknown, options: Record<string, (...args: never[]) => unknown>) => {
      visits.push({ method, url, data })
      answer(options)
    }) as never)
  }
  vi.spyOn(router, 'delete').mockImplementation(((url: string, options: Record<string, unknown>) => {
    visits.push({ method: 'delete', url, data: options.data })
    answer(options as Record<string, (...args: never[]) => unknown>)
  }) as never)
  return visits
}

let context: FakeModelContext

beforeEach(() => {
  context = installModelContext()
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('a form tool must not fetch'))))
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  removeModelContext()
})

test('agent_015_react_component_tool_registers_while_mounted', async () => {
  const view = probe(
    ({ total }: { total: number }) =>
      usePageTool<{ amount: number }>({
        name: 'cart.add',
        title: 'Add to the cart',
        description: 'Add an amount to the cart total.',
        inputSchema: SCHEMA,
        consequential: true,
        execute: ({ amount }) => ({ total: total + amount }),
      }),
    { total: 1 },
  )
  expect(context.names()).toEqual(['cart.add'])
  expect(context.descriptors[0].annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
  })
  expect(context.descriptors[0].title).toBe('Add to the cart')
  expect(await context.call('cart.add', { amount: 2 })).toEqual({
    content: [{ type: 'text', text: '{"total":3}' }],
    structuredContent: { total: 3 },
    isError: false,
  })
  // A new closure on the next render answers the next call, and the tool
  // is not registered again for it.
  view.rerender({ total: 10 })
  expect((await context.call('cart.add', { amount: 2 })).structuredContent).toEqual({ total: 12 })
  expect(context.descriptors).toHaveLength(1)
  view.unmount()
})

test('agent_015_react_component_tool_leaves_on_unmount', async () => {
  const view = probe(
    () =>
      usePageTool({
        name: 'cart.clear',
        description: 'Empty the cart.',
        inputSchema: { type: 'object' },
        execute: () => {
          throw new Error('The cart is locked.\nTry later.')
        },
      }),
    {},
  )
  expect(await context.call('cart.clear', {})).toEqual({
    content: [{ type: 'text', text: 'The cart is locked. Try later.' }],
    isError: true,
  })
  view.unmount()
  expect(context.names()).toEqual([])
})

test('agent_015_react_form_tool_submits_through_inertia', async () => {
  const visits = recordVisits((options) => {
    options.onSuccess?.({ component: 'Notes/Show', url: '/notes/1', props: {} } as never)
    options.onFinish?.({} as never)
  })
  let form!: ReturnType<typeof useForm<{ title: string; body: string | null }>>
  const view = probe(() => {
    form = useForm<{ title: string; body: string | null }>({ title: '', body: null })
    useFormTool(form, {
      name: 'notes.create',
      description: 'Write a note.',
      schema: toolSchemas.StoreNoteRequest,
      action: { method: 'post', url: '/notes' },
    })
  }, {})
  expect(context.descriptors[0].inputSchema).toBe(toolSchemas.StoreNoteRequest)
  let result!: Awaited<ReturnType<FakeModelContext['call']>>
  await act(async () => {
    result = await context.call('notes.create', { title: 'Hello', body: 'World', stray: 1 })
  })
  expect(visits).toEqual([{ method: 'post', url: '/notes', data: { title: 'Hello', body: 'World' } }])
  expect(result).toEqual({
    content: [{ type: 'text', text: '{"outcome":"submitted","component":"Notes/Show","url":"/notes/1"}' }],
    structuredContent: { outcome: 'submitted', component: 'Notes/Show', url: '/notes/1' },
    isError: false,
  })
  expect(form.data).toEqual({ title: 'Hello', body: 'World' })
  expect(fetch).not.toHaveBeenCalled()
  view.unmount()
  expect(context.names()).toEqual([])
})

test('agent_015_react_form_tool_validation_errors_are_an_error_result', async () => {
  recordVisits((options) => {
    options.onError?.({ title: 'Give the note a title of 1 to 255 characters.', body: 'Keep the note to 10000 characters.' } as never)
    options.onFinish?.({} as never)
  })
  const view = probe(() => {
    const form = useForm({ title: '', body: '' })
    useFormTool(form, {
      name: 'notes.create',
      description: 'Write a note.',
      schema: toolSchemas.StoreNoteRequest,
      action: { method: 'post', url: '/notes' },
    })
  }, {})
  let result!: Awaited<ReturnType<FakeModelContext['call']>>
  await act(async () => {
    result = await context.call('notes.create', { title: '' })
  })
  expect(result).toEqual({
    content: [
      {
        type: 'text',
        text: 'The input is not valid:\n- title: Give the note a title of 1 to 255 characters.\n- body: Keep the note to 10000 characters.',
      },
    ],
    structuredContent: {
      outcome: 'invalid',
      errors: { title: 'Give the note a title of 1 to 255 characters.', body: 'Keep the note to 10000 characters.' },
    },
    isError: true,
  })
  view.unmount()
})

test('agent_015_react_destructive_form_tool_fills_and_does_not_submit', async () => {
  const visits = recordVisits(() => undefined)
  let form!: ReturnType<typeof useForm<{ confirm: string }>>
  const view = probe(() => {
    form = useForm({ confirm: '' })
    useFormTool(form, {
      name: 'account.delete',
      description: 'Delete the account.',
      schema: { type: 'object', properties: { confirm: { type: 'string' } } },
      action: { method: 'delete', url: '/account' },
    })
    useFormTool(form, {
      name: 'notes.purge',
      description: 'Purge the notes.',
      schema: { type: 'object', properties: { confirm: { type: 'string' } } },
      action: { method: 'post', url: '/notes/purge' },
      destructive: true,
    })
  }, {})
  for (const name of ['account.delete', 'notes.purge']) {
    const descriptor = context.descriptors.find((tool) => tool.name === name)
    expect(descriptor?.annotations.consequentialHint).toBe(true)
    let result!: Awaited<ReturnType<FakeModelContext['call']>>
    await act(async () => {
      result = await context.call(name, { confirm: name })
    })
    expect(result).toEqual({
      content: [{ type: 'text', text: 'The form is filled. The user reviews it and submits it.' }],
      structuredContent: { outcome: 'filled', submitted: false },
      isError: false,
    })
    expect(form.data).toEqual({ confirm: name })
  }
  expect(visits).toEqual([])
  view.unmount()
})
