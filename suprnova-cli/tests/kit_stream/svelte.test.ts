import { flushSync, mount, unmount as destroy } from 'svelte'
import { get } from 'svelte/store'
import { afterEach, expect, test, vi } from 'vitest'
import StreamProbe from './StreamProbe.svelte'
import StreamReaders from './StreamReaders.svelte'
import { csrfHeaders, useEventStream, useJsonEventStream, useJsonStream, useStream } from './src/lib/stream'
import { FakeEventSource, fakeFetch, settle } from './fakes'

function probe<T>(hook: () => T) {
  const result = { current: undefined as unknown as T }
  const component = mount(StreamProbe, {
    target: document.body,
    props: {
      setup: () => {
        result.current = hook()
      },
    },
  })
  flushSync()
  return { result, unmount: () => destroy(component) }
}

const step = async (work: () => void = () => undefined) => {
  work()
  await settle()
}

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
  expect(get(result.current).isFetching).toBe(false)

  await step(() => void result.current.send({ message: 'hi' }))
  expect(get(result.current).isFetching).toBe(true)
  const request = server.requests[0]
  const headers = request.init.headers as Record<string, string>
  expect(request.url).toBe('/chat')
  expect(request.init.method).toBe('POST')
  expect(headers['X-CSRF-TOKEN']).toBe('token-1')
  expect(headers['X-STREAM-ID']).toBe(result.current.id)
  expect(JSON.parse(String(request.init.body))).toEqual({ message: 'hi' })

  await step(() => request.respond())
  expect(get(result.current).isFetching).toBe(false)
  expect(get(result.current).isStreaming).toBe(true)
  await step(() => request.push('Hel'))
  expect(get(result.current).data).toBe('Hel')
  await step(() => {
    request.push('lo')
    request.end()
  })
  expect(get(result.current).data).toBe('Hello')
  expect(get(result.current).isStreaming).toBe(false)
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
  expect(get(result.current).isStreaming).toBe(false)
  expect(onCancel).toHaveBeenCalledTimes(1)

  await step(() => void result.current.send())
  expect(get(result.current).data).toBe('')
  unmount()
  expect(server.requests[1].aborted()).toBe(true)
})

test('useJsonStream parses the whole response once it ends', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const { result, unmount } = probe(() => useJsonStream<Array<{ id: number }>>('/export'))
  await step(() => void result.current.send())
  await step(() => server.requests[0].respond('application/json'))
  await step(() => server.requests[0].push('[{"id":1},'))
  expect(get(result.current).data).toBe(null)
  expect(get(result.current).strData).toBe('[{"id":1},')
  await step(() => {
    server.requests[0].push('{"id":2}]')
    server.requests[0].end()
  })
  expect(get(result.current).data).toEqual([{ id: 1 }, { id: 2 }])
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
  expect(get(result.current).isFetching).toBe(true)

  await step(() => request.respond('text/event-stream'))
  expect(get(result.current).isStreaming).toBe(true)
  await step(() => request.push('event: update\ndata: {"n":1}\n\n'))
  expect(get(result.current).events).toEqual([{ n: 1 }])
  await step(() => request.push('data: {"n":2}\n\ndata: </stream>\n\n'))
  expect(get(result.current).events).toEqual([{ n: 1 }, { n: 2 }])
  expect(get(result.current).isStreaming).toBe(false)
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
  expect(get(result.current).isFetching).toBe(false)
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
  expect(get(result.current).message).toBe('one two')
  expect(get(result.current).messageParts).toEqual(['one', 'two'])
  await step(() => source.emit('update', '</stream>'))
  expect(source.closed).toBe(true)
  expect(onComplete).toHaveBeenCalledTimes(1)
  unmount()

  const second = probe(() => useEventStream('/progress'))
  const next = FakeEventSource.opened.at(-1)!
  second.unmount()
  expect(next.closed).toBe(true)
})


test('CSRF headers prefer the fresh cookie over a stale meta token (#181)', () => {
  document.head.innerHTML = '<meta name="csrf-token" content="stale-token">'
  document.cookie = 'XSRF-TOKEN=fresh%2Btoken; path=/'
  try {
    expect(csrfHeaders()).toEqual({ 'X-XSRF-TOKEN': 'fresh+token' })
    expect(csrfHeaders({ csrfToken: 'explicit-token' })).toEqual({
      'X-CSRF-TOKEN': 'explicit-token',
    })
  } finally {
    document.cookie = 'XSRF-TOKEN=; Max-Age=0; path=/'
  }
  expect(csrfHeaders()).toEqual({ 'X-CSRF-TOKEN': 'stale-token' })
})

test('CSRF headers use the configured cookie and header names (#181)', () => {
  document.head.innerHTML = '<meta name="csrf-token" content="stale-token">'
  document.cookie = 'CUSTOM-XSRF=custom%2Ftoken; path=/'
  try {
    expect(csrfHeaders({ xsrfCookieName: 'CUSTOM-XSRF', xsrfHeaderName: 'X-CUSTOM-XSRF' }))
      .toEqual({ 'X-CUSTOM-XSRF': 'custom/token' })
  } finally {
    document.cookie = 'CUSTOM-XSRF=; Max-Age=0; path=/'
  }
})


test('readers mounted together share text and callbacks for one id (#182)', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  const onData = [vi.fn(), vi.fn()]
  const onFinish = [vi.fn(), vi.fn()]
  const readers: Array<ReturnType<typeof useStream>> = []
  const component = mount(StreamReaders, {
    target: document.body,
    props: {
      readers: onData.map((callback, index) => () => {
        readers[index] = useStream('/chat', {
          id: 'shared-readers', onData: callback, onFinish: onFinish[index],
        })
      }),
    },
  })
  flushSync()
  try {
    await step(() => void readers[0].send())
    await step(() => server.requests[0].respond())
    await step(() => server.requests[0].push('shared'))
    expect(readers.map((reader) => get(reader).data)).toEqual(['shared', 'shared'])
    for (const callback of onData) expect(callback).toHaveBeenCalledWith('shared')
    await step(() => server.requests[0].end())
    for (const callback of onFinish) expect(callback).toHaveBeenCalledTimes(1)
  } finally {
    await destroy(component)
  }
})
