// The Vue kit's agent composables in jsdom, against a fake model context
// and Inertia's own `useForm` with the router's visits recorded (AGENT-015).

import { router, useForm } from '@inertiajs/vue3'
import { createApp, defineComponent } from 'vue'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { useFormTool, usePageTool } from './src/lib/agents'
import { toolSchemas } from './src/types/tool-schemas'
import { installModelContext, removeModelContext, settle, type FakeModelContext } from './fakes'

const SCHEMA = { type: 'object' as const, properties: { amount: { type: 'integer' } } }

function probe(setup: () => void) {
  const app = createApp(
    defineComponent({
      setup() {
        setup()
        return () => null
      },
    }),
  )
  app.mount(document.createElement('div'))
  return { unmount: () => app.unmount() }
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

test('agent_015_vue_component_tool_registers_while_mounted', async () => {
  let total = 1
  const view = probe(() =>
    usePageTool<{ amount: number }>({
      name: 'cart.add',
      title: 'Add to the cart',
      description: 'Add an amount to the cart total.',
      inputSchema: SCHEMA,
      consequential: true,
      execute: ({ amount }) => ({ total: total + amount }),
    }),
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
  total = 10
  expect((await context.call('cart.add', { amount: 2 })).structuredContent).toEqual({ total: 12 })
  expect(context.descriptors).toHaveLength(1)
  view.unmount()
})

test('agent_015_vue_component_tool_leaves_on_unmount', async () => {
  const view = probe(() =>
    usePageTool({
      name: 'cart.clear',
      description: 'Empty the cart.',
      inputSchema: { type: 'object' },
      execute: () => {
        throw new Error('The cart is locked.\nTry later.')
      },
    }),
  )
  expect(await context.call('cart.clear', {})).toEqual({
    content: [{ type: 'text', text: 'The cart is locked. Try later.' }],
    isError: true,
  })
  view.unmount()
  expect(context.names()).toEqual([])
})

test('agent_015_vue_form_tool_submits_through_inertia', async () => {
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
  })
  expect(context.descriptors[0].inputSchema).toBe(toolSchemas.StoreNoteRequest)
  const result = await context.call('notes.create', { title: 'Hello', body: 'World', stray: 1 })
  await settle()
  expect(visits).toEqual([{ method: 'post', url: '/notes', data: { title: 'Hello', body: 'World' } }])
  expect(result).toEqual({
    content: [{ type: 'text', text: '{"outcome":"submitted","component":"Notes/Show","url":"/notes/1"}' }],
    structuredContent: { outcome: 'submitted', component: 'Notes/Show', url: '/notes/1' },
    isError: false,
  })
  expect(form.data()).toEqual({ title: 'Hello', body: 'World' })
  expect(fetch).not.toHaveBeenCalled()
  view.unmount()
  expect(context.names()).toEqual([])
})

test('agent_015_vue_form_tool_validation_errors_are_an_error_result', async () => {
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
  })
  const result = await context.call('notes.create', { title: '' })
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

test('agent_015_vue_destructive_form_tool_fills_and_does_not_submit', async () => {
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
  })
  for (const name of ['account.delete', 'notes.purge']) {
    const descriptor = context.descriptors.find((tool) => tool.name === name)
    expect(descriptor?.annotations.consequentialHint).toBe(true)
    const result = await context.call(name, { confirm: name })
    expect(result).toEqual({
      content: [{ type: 'text', text: 'The form is filled. The user reviews it and submits it.' }],
      structuredContent: { outcome: 'filled', submitted: false },
      isError: false,
    })
    expect(form.data()).toEqual({ confirm: name })
  }
  expect(visits).toEqual([])
  view.unmount()
})
