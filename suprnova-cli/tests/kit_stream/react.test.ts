import { act, createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, test, vi } from 'vitest'
import { useEventStream, useJsonEventStream, useJsonStream, useStream } from './src/lib/stream'
import { FakeEventSource, fakeFetch, settle } from './fakes'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

function probe<T>(hook: () => T) {
  const result = { current: undefined as unknown as T }
  const Probe = () => {
    result.current = hook()
    return null
  }
  const root = createRoot(document.createElement('div'))
  act(() => root.render(createElement(Probe)))
  return { result, unmount: () => act(() => root.unmount()) }
}

const step = (work: () => void = () => undefined) =>
  act(async () => {
    work()
    await settle()
  })

afterEach(() => {
  vi.unstubAllGlobals()
  document.head.innerHTML = ''
})

test('useStream posts, streams the text as it arrives and finishes', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  document.head.innerHTML = '<meta name="csrf-token" content="token-1">'
  const onFinish = vi.fn()
  const { result, unmount } = probe(() => useStream<{ message: string }>('/chat', { onFinish }))
  expect(result.current.isFetching).toBe(false)

  await step(() => void result.current.send({ message: 'hi' }))
  expect(result.current.isFetching).toBe(true)
  const request = server.requests[0]
  const headers = request.init.headers as Record<string, string>
  expect(request.url).toBe('/chat')
  expect(request.init.method).toBe('POST')
  expect(headers['X-CSRF-TOKEN']).toBe('token-1')
  expect(headers['X-STREAM-ID']).toBe(result.current.id)
  expect(JSON.parse(String(request.init.body))).toEqual({ message: 'hi' })

  await step(() => request.respond())
  expect(result.current.isFetching).toBe(false)
  expect(result.current.isStreaming).toBe(true)
  await step(() => request.push('Hel'))
  expect(result.current.data).toBe('Hel')
  await step(() => {
    request.push('lo')
    request.end()
  })
  expect(result.current.data).toBe('Hello')
  expect(result.current.isStreaming).toBe(false)
  expect(onFinish).toHaveBeenCalledTimes(1)
  unmount()
})

test('useStream cancels on request and when its component unmounts', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const onCancel = vi.fn()
  const { result, unmount } = probe(() => useStream('/chat', { onCancel }))

  await step(() => void result.current.send())
  await step(() => server.requests[0].respond())
  await step(() => server.requests[0].push('partial'))
  await step(() => result.current.cancel())
  expect(server.requests[0].aborted()).toBe(true)
  expect(result.current.isStreaming).toBe(false)
  expect(onCancel).toHaveBeenCalledTimes(1)

  await step(() => void result.current.send())
  expect(result.current.data).toBe('')
  unmount()
  expect(server.requests[1].aborted()).toBe(true)
})

test('useJsonStream parses the whole response once it ends', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const { result, unmount } = probe(() =>
    useJsonStream<Array<{ id: number }>>('/export'),
  )
  await step(() => void result.current.send())
  await step(() => server.requests[0].respond('application/json'))
  await step(() => server.requests[0].push('[{"id":1},'))
  expect(result.current.data).toBe(null)
  expect(result.current.strData).toBe('[{"id":1},')
  await step(() => {
    server.requests[0].push('{"id":2}]')
    server.requests[0].end()
  })
  expect(result.current.data).toEqual([{ id: 1 }, { id: 2 }])
  unmount()
})

test('useJsonEventStream collects each event as JSON until the end signal', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const onEvent = vi.fn()
  const onFinish = vi.fn()
  const { result, unmount } = probe(() =>
    useJsonEventStream<{ n: number }, { environment: string }>('/deploy', { onEvent, onFinish }),
  )

  await step(() => void result.current.send({ environment: 'production' }))
  const request = server.requests[0]
  expect((request.init.headers as Record<string, string>).Accept).toBe('text/event-stream')
  expect(JSON.parse(String(request.init.body))).toEqual({ environment: 'production' })
  expect(result.current.isFetching).toBe(true)

  await step(() => request.respond('text/event-stream'))
  expect(result.current.isStreaming).toBe(true)
  await step(() => request.push('event: update\ndata: {"n":1}\n\n'))
  expect(result.current.events).toEqual([{ n: 1 }])
  await step(() => request.push('data: {"n":2}\n\ndata: </stream>\n\n'))
  expect(result.current.events).toEqual([{ n: 1 }, { n: 2 }])
  expect(result.current.isStreaming).toBe(false)
  expect(onEvent).toHaveBeenCalledTimes(2)
  expect(onFinish).toHaveBeenCalledTimes(1)
  unmount()
})

test('useJsonEventStream reports an error status once', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const onError = vi.fn()
  const { result, unmount } = probe(() => useJsonEventStream('/deploy', { onError }))
  await step(() => void result.current.send())
  await step(() => server.requests[0].respond('application/json', 422, '{"errors":{}}'))
  expect(onError).toHaveBeenCalledTimes(1)
  expect((onError.mock.calls[0][0] as { status: number }).status).toBe(422)
  expect(result.current.isFetching).toBe(false)
  unmount()
})

test('useEventStream joins messages, closes at the end signal and on unmount', async () => {
  vi.stubGlobal('EventSource', FakeEventSource)
  const onComplete = vi.fn()
  const { result, unmount } = probe(() => useEventStream('/progress', { onComplete }))
  const source = FakeEventSource.opened.at(-1)!
  expect(source.url).toBe('/progress')

  await step(() => {
    source.emit('update', 'one')
    source.emit('update', 'two')
  })
  expect(result.current.message).toBe('one two')
  expect(result.current.messageParts).toEqual(['one', 'two'])
  await step(() => source.emit('update', '</stream>'))
  expect(source.closed).toBe(true)
  expect(onComplete).toHaveBeenCalledTimes(1)
  unmount()

  const second = probe(() => useEventStream('/progress'))
  const next = FakeEventSource.opened.at(-1)!
  second.unmount()
  expect(next.closed).toBe(true)
})
