// React hooks for the responses Suprnova streams, over the engine in
// `core.ts`. Each hook keeps Laravel's `@laravel/stream-react` name, options
// and result, so Laravel's stream documentation applies:
//
//   useStream('/chat')          HttpResponse::stream_bytes, read as text
//   useJsonStream('/export')    HttpResponse::stream_json, parsed when it ends
//   useEventStream('/progress') HttpResponse::event_stream, over a GET
//   useJsonEventStream('/deploy') HttpResponse::event_stream, after a POST

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import {
  JsonEventStream,
  openEventStream,
  streamId,
  textStream,
  type EventStreamConnection,
  type EventStreamOptions,
  type EventStreamState,
  type JsonEventStreamOptions,
  type StreamOptions,
} from './core'

export {
  csrfHeaders,
  readEventStream,
  streamJsonEvents,
  StreamResponseError,
  type CsrfHeaderOptions,
  type EventStreamOptions,
  type JsonEventStreamOptions,
  type JsonEventStreamRequest,
  type StreamFrame,
  type StreamOptions,
} from './core'

/** Cancels `cancel` when the page unloads, until the component unmounts. */
function useCancelOnUnload(cancel: () => void): void {
  useEffect(() => {
    window.addEventListener('beforeunload', cancel)
    return () => window.removeEventListener('beforeunload', cancel)
  }, [cancel])
}

/**
 * POSTs a JSON body with `send` and reads the response as text as it
 * arrives. Components that pass the same `id` share one stream.
 */
export function useStream<TSendBody extends object = {}, TJsonData = null>(
  url: string,
  options: StreamOptions<TSendBody> = {},
) {
  const [id] = useState(() => options.id ?? streamId())
  const [stream] = useState(() => textStream<TJsonData>(id))
  const state = useSyncExternalStore(stream.subscribe, stream.snapshot, stream.snapshot)
  const latest = useRef(options)
  latest.current = options

  useEffect(
    () =>
      stream.listen({
        onResponse: (response) => latest.current.onResponse?.(response),
        onData: (data) => latest.current.onData?.(data),
        onCancel: () => latest.current.onCancel?.(),
        onFinish: () => latest.current.onFinish?.(),
        onError: (error) => latest.current.onError?.(error),
        onBeforeSend: (request) => latest.current.onBeforeSend?.(request),
      }),
    [stream],
  )
  useCancelOnUnload(stream.cancel)

  const send = useCallback(
    (body?: TSendBody) => stream.send(url, body, latest.current),
    [stream, url],
  )
  useEffect(() => {
    if (latest.current.initialInput) {
      void send(latest.current.initialInput)
    }
    // Once, on mount.
  }, [])

  return {
    data: state.data,
    jsonData: state.jsonData,
    isFetching: state.isFetching,
    isStreaming: state.isStreaming,
    id,
    send,
    cancel: stream.cancel,
    clearData: stream.clearData,
  }
}

/** `useStream` for a `stream_json` response: `data` is the parsed JSON once it ends. */
export function useJsonStream<TJsonData = null, TSendBody extends object = {}>(
  url: string,
  options: Omit<StreamOptions<TSendBody>, 'json'> = {},
) {
  const { data, jsonData, ...rest } = useStream<TSendBody, TJsonData>(url, {
    ...options,
    json: true,
  })
  return { data: jsonData, strData: data, ...rest }
}

/** Listens to an event stream over a GET from mount until its end signal. */
export function useEventStream(url: string, options: EventStreamOptions = {}) {
  const [state, setState] = useState<EventStreamState>({ message: '', messageParts: [] })
  const latest = useRef(options)
  latest.current = options
  const connection = useRef<EventStreamConnection | null>(null)
  const names = [options.eventName ?? 'update'].flat().join('\n')

  useEffect(() => {
    setState({ message: '', messageParts: [] })
    const opened = openEventStream(url, () => latest.current, setState)
    connection.current = opened
    return () => opened.close()
  }, [url, names])

  const close = useCallback((resetMessage = false) => connection.current?.close(resetMessage), [])
  const clearMessage = useCallback(() => connection.current?.clear(), [])

  return { message: state.message, messageParts: state.messageParts, close, clearMessage }
}

/** POSTs a JSON body with `send` and collects the stream's events, each parsed as JSON. */
export function useJsonEventStream<TEvent = unknown, TSendBody extends object = {}>(
  url: string,
  options: JsonEventStreamOptions<TEvent, TSendBody> = {},
) {
  const latest = useRef(options)
  latest.current = options
  const [stream] = useState(() => new JsonEventStream<TEvent, TSendBody>(() => latest.current))
  const state = useSyncExternalStore(stream.subscribe, stream.snapshot, stream.snapshot)

  useCancelOnUnload(stream.cancel)
  useEffect(() => stream.cancel, [stream])

  const send = useCallback((body?: TSendBody) => stream.send(url, body), [stream, url])
  useEffect(() => {
    if (latest.current.initialInput) {
      void send(latest.current.initialInput)
    }
    // Once, on mount.
  }, [])

  return {
    events: state.events,
    isFetching: state.isFetching,
    isStreaming: state.isStreaming,
    send,
    cancel: stream.cancel,
    clearEvents: stream.clearEvents,
  }
}
