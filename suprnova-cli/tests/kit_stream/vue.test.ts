import { createApp, defineComponent, h } from 'vue'
import { afterEach, expect, test, vi } from 'vitest'
import { csrfHeaders, useEventStream, useJsonEventStream, useJsonStream, useStream } from './src/lib/stream'
import { FakeEventSource, fakeFetch, settle } from './fakes'

function probe<T>(hook: () => T) {
  const result = { current: undefined as unknown as T }
  const app = createApp(
    defineComponent({
      setup() {
        result.current = hook()
        return () => null
      },
    }),
  )
  app.mount(document.createElement('div'))
  return { result, unmount: () => app.unmount() }
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
  expect(result.current.isFetching.value).toBe(false)

  await step(() => void result.current.send({ message: 'hi' }))
  expect(result.current.isFetching.value).toBe(true)
  const request = server.requests[0]
  const headers = request.init.headers as Record<string, string>
  expect(request.url).toBe('/chat')
  expect(request.init.method).toBe('POST')
  expect(headers['X-CSRF-TOKEN']).toBe('token-1')
  expect(headers['X-STREAM-ID']).toBe(result.current.id)
  expect(JSON.parse(String(request.init.body))).toEqual({ message: 'hi' })

  await step(() => request.respond())
  expect(result.current.isFetching.value).toBe(false)
  expect(result.current.isStreaming.value).toBe(true)
  await step(() => request.push('Hel'))
  expect(result.current.data.value).toBe('Hel')
  await step(() => {
    request.push('lo')
    request.end()
  })
  expect(result.current.data.value).toBe('Hello')
  expect(result.current.isStreaming.value).toBe(false)
  expect(onFinish).toHaveBeenCalledTimes(1)
  unmount()
})

test('useStream reads a ref URL when it sends', async () => {
  const server = fakeFetch()
  vi.stubGlobal('fetch', server.fetch)
  let path = '/first'
  const { result, unmount } = probe(() => useStream(() => path))
  path = '/second'
  await step(() => void result.current.send())
  expect(server.requests[0].url).toBe('/second')
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
  expect(result.current.isStreaming.value).toBe(false)
  expect(onCancel).toHaveBeenCalledTimes(1)

  await step(() => void result.current.send())
  expect(result.current.data.value).toBe('')
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
  expect(result.current.data.value).toBe(null)
  expect(result.current.strData.value).toBe('[{"id":1},')
  await step(() => {
    server.requests[0].push('{"id":2}]')
    server.requests[0].end()
  })
  expect(result.current.data.value).toEqual([{ id: 1 }, { id: 2 }])
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
  expect(result.current.isFetching.value).toBe(true)

  await step(() => request.respond('text/event-stream'))
  expect(result.current.isStreaming.value).toBe(true)
  await step(() => request.push('event: update\ndata: {"n":1}\n\n'))
  expect(result.current.events.value).toEqual([{ n: 1 }])
  await step(() => request.push('data: {"n":2}\n\ndata: </stream>\n\n'))
  expect(result.current.events.value).toEqual([{ n: 1 }, { n: 2 }])
  expect(result.current.isStreaming.value).toBe(false)
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
  expect(result.current.isFetching.value).toBe(false)
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
  expect(result.current.message.value).toBe('one two')
  expect(result.current.messageParts.value).toEqual(['one', 'two'])
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
  const Reader = defineComponent({
    props: { index: { type: Number, required: true } },
    setup(props) {
      readers[props.index] = useStream('/chat', {
        id: 'shared-readers', onData: onData[props.index], onFinish: onFinish[props.index],
      })
      return () => null
    },
  })
  const app = createApp({ render: () => [h(Reader, { index: 0 }), h(Reader, { index: 1 })] })
  app.mount(document.createElement('div'))
  try {
    await step(() => void readers[0].send())
    await step(() => server.requests[0].respond())
    await step(() => server.requests[0].push('shared'))
    expect(readers.map((reader) => reader.data.value)).toEqual(['shared', 'shared'])
    for (const callback of onData) expect(callback).toHaveBeenCalledWith('shared')
    await step(() => server.requests[0].end())
    for (const callback of onFinish) expect(callback).toHaveBeenCalledTimes(1)
  } finally {
    app.unmount()
  }
})


test('EventSource stays open after an error and receives later messages (#183)', async () => {
  vi.stubGlobal('EventSource', FakeEventSource)
  const onError = vi.fn()
  const onComplete = vi.fn()
  const { result, unmount } = probe(() => useEventStream('/progress', { onError, onComplete }))
  const source = FakeEventSource.opened.at(-1)!
  const error = new Event('error')
  try {
    await step(() => source.dispatchEvent(error))
    expect(onError).toHaveBeenCalledWith(error)
    expect(source.closed).toBe(false)
    expect(onComplete).not.toHaveBeenCalled()
    await step(() => source.emit('update', 'reconnected'))
    expect(result.current.message.value).toBe('reconnected')
    await step(() => source.emit('update', '</stream>'))
    expect(source.closed).toBe(true)
    expect(onComplete).toHaveBeenCalledTimes(1)
    await step(() => source.emit('update', 'after-end'))
    expect(result.current.message.value).toBe('reconnected')
  } finally {
    await unmount()
  }

  const second = probe(() => useEventStream('/progress'))
  const next = FakeEventSource.opened.at(-1)!
  await step(() => next.dispatchEvent(new Event('error')))
  await second.unmount()
  expect(next.closed).toBe(true)
})

test('EventSource caller cancellation still closes the source (#183)', async () => {
  vi.stubGlobal('EventSource', FakeEventSource)
  const { result, unmount } = probe(() => useEventStream('/progress'))
  const source = FakeEventSource.opened.at(-1)!
  await step(() => result.current.close())
  expect(source.closed).toBe(true)
  await unmount()
})
