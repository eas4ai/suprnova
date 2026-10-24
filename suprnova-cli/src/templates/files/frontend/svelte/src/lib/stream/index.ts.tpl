// Svelte stores for the responses Suprnova streams, over the engine in
// `core.ts`. Each keeps Laravel's `@laravel/stream-svelte` name, options and
// result, so Laravel's stream documentation applies: call it while the
// component initializes and read it as `$stream` in the markup.
//
//   useStream('/chat')          HttpResponse::stream_bytes, read as text
//   useJsonStream('/export')    HttpResponse::stream_json, parsed when it ends
//   useEventStream('/progress') HttpResponse::event_stream, over a GET
//   useJsonEventStream('/deploy') HttpResponse::event_stream, after a POST
//
// A store connects when its component mounts, so the server render opens
// no request.

import { onMount } from 'svelte'
import { derived, readable, writable, type Readable } from 'svelte/store'
import {
  JsonEventStream,
  openEventStream,
  streamId,
  textStream,
  type EventStreamConnection,
  type EventStreamOptions,
  type EventStreamState,
  type JsonEventStreamOptions,
  type JsonEventStreamState,
  type StreamOptions,
  type StreamState,
} from './core'

export {
  csrfHeaders,
  readEventStream,
  streamJsonEvents,
  StreamResponseError,
  type CsrfHeaderOptions,
  type EventStreamOptions,
  type EventStreamState,
  type JsonEventStreamOptions,
  type JsonEventStreamRequest,
  type JsonEventStreamState,
  type StreamFrame,
  type StreamOptions,
  type StreamState,
} from './core'

/** A URL, or a function that answers it when a request starts. */
type Url = string | (() => string)

const resolve = (url: Url): string => (typeof url === 'function' ? url() : url)

/** What `useStream` returns: a store of the stream's state and its controls. */
export type Stream<TJsonData = null, TSendBody extends object = {}> = Readable<
  StreamState<TJsonData>
> & {
  id: string
  send: (body?: TSendBody) => Promise<void>
  cancel: () => void
  clearData: () => void
}

/**
 * POSTs a JSON body with `send` and reads the response as text as it
 * arrives. Components that pass the same `id` share one stream.
 */
export function useStream<TSendBody extends object = {}, TJsonData = null>(
  url: Url,
  options: StreamOptions<TSendBody> = {},
): Stream<TJsonData, TSendBody> {
  const id = options.id ?? streamId()
  const stream = textStream<TJsonData>(id)
  const store = readable(stream.snapshot(), (set) => {
    const unsubscribe = stream.subscribe(() => set(stream.snapshot()))
    set(stream.snapshot())
    return unsubscribe
  })
  const send = (body?: TSendBody) => stream.send(resolve(url), body, options)

  onMount(() => {
    // Holds the stream while the component lives, whether or not the
    // markup reads it.
    const hold = store.subscribe(() => undefined)
    const stopListening = stream.listen(options)
    window.addEventListener('beforeunload', stream.cancel)
    if (options.initialInput) {
      void send(options.initialInput)
    }
    return () => {
      window.removeEventListener('beforeunload', stream.cancel)
      stopListening()
      hold()
    }
  })

  return {
    subscribe: store.subscribe,
    id,
    send,
    cancel: stream.cancel,
    clearData: stream.clearData,
  }
}

/** The state `useJsonStream` holds: `data` is the parsed JSON once it ends. */
export type JsonStreamState<TJsonData = null> = {
  data: TJsonData | null
  strData: string
  isFetching: boolean
  isStreaming: boolean
}

/** `useStream` for a `stream_json` response. */
export function useJsonStream<TJsonData = null, TSendBody extends object = {}>(
  url: Url,
  options: Omit<StreamOptions<TSendBody>, 'json'> = {},
) {
  const stream = useStream<TSendBody, TJsonData>(url, { ...options, json: true })
  const store = derived<Stream<TJsonData, TSendBody>, JsonStreamState<TJsonData>>(
    stream,
    (state) => ({
      data: state.jsonData,
      strData: state.data,
      isFetching: state.isFetching,
      isStreaming: state.isStreaming,
    }),
  )
  return {
    subscribe: store.subscribe,
    id: stream.id,
    send: stream.send,
    cancel: stream.cancel,
    clearData: stream.clearData,
  }
}

/** Listens to an event stream over a GET from mount until its end signal. */
export function useEventStream(url: Url, options: EventStreamOptions = {}) {
  const store = writable<EventStreamState>({ message: '', messageParts: [] })
  let connection: EventStreamConnection | null = null

  onMount(() => {
    connection = openEventStream(resolve(url), () => options, store.set)
    return () => connection?.close()
  })

  return {
    subscribe: store.subscribe,
    close: (resetMessage = false) => connection?.close(resetMessage),
    clearMessage: () => connection?.clear(),
  }
}

/** POSTs a JSON body with `send` and collects the stream's events, each parsed as JSON. */
export function useJsonEventStream<TEvent = unknown, TSendBody extends object = {}>(
  url: Url,
  options: JsonEventStreamOptions<TEvent, TSendBody> = {},
) {
  const stream = new JsonEventStream<TEvent, TSendBody>(() => options)
  const store = readable<JsonEventStreamState<TEvent>>(stream.snapshot(), (set) => {
    set(stream.snapshot())
    return stream.subscribe(() => set(stream.snapshot()))
  })
  const send = (body?: TSendBody) => stream.send(resolve(url), body)

  onMount(() => {
    window.addEventListener('beforeunload', stream.cancel)
    if (options.initialInput) {
      void send(options.initialInput)
    }
    return () => {
      window.removeEventListener('beforeunload', stream.cancel)
      stream.cancel()
    }
  })

  return {
    subscribe: store.subscribe,
    send,
    cancel: stream.cancel,
    clearEvents: stream.clearEvents,
  }
}
