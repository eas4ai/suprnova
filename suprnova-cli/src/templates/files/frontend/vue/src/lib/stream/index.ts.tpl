// Vue composables for the responses Suprnova streams, over the engine in
// `core.ts`. Each keeps Laravel's `@laravel/stream-vue` name, options and
// result, so Laravel's stream documentation applies:
//
//   useStream('/chat')          HttpResponse::stream_bytes, read as text
//   useJsonStream('/export')    HttpResponse::stream_json, parsed when it ends
//   useEventStream('/progress') HttpResponse::event_stream, over a GET
//   useJsonEventStream('/deploy') HttpResponse::event_stream, after a POST
//
// A composable connects when its component mounts, so the server render
// opens no request.

import {
  computed,
  onBeforeUnmount,
  onMounted,
  shallowRef,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from 'vue'
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

/**
 * POSTs a JSON body with `send` and reads the response as text as it
 * arrives. Components that pass the same `id` share one stream.
 */
export function useStream<TSendBody extends object = {}, TJsonData = null>(
  url: MaybeRefOrGetter<string>,
  options: StreamOptions<TSendBody> = {},
) {
  const id = options.id ?? streamId()
  const stream = textStream<TJsonData>(id)
  const state = shallowRef(stream.snapshot())
  const send = (body?: TSendBody) => stream.send(toValue(url), body, options)
  let stop: Array<() => void> = []

  onMounted(() => {
    stop = [
      stream.subscribe(() => {
        state.value = stream.snapshot()
      }),
      stream.listen(options),
    ]
    state.value = stream.snapshot()
    window.addEventListener('beforeunload', stream.cancel)
    if (options.initialInput) {
      void send(options.initialInput)
    }
  })
  onBeforeUnmount(() => {
    window.removeEventListener('beforeunload', stream.cancel)
    for (const end of stop) {
      end()
    }
  })

  return {
    data: computed(() => state.value.data),
    jsonData: computed(() => state.value.jsonData),
    isFetching: computed(() => state.value.isFetching),
    isStreaming: computed(() => state.value.isStreaming),
    id,
    send,
    cancel: stream.cancel,
    clearData: stream.clearData,
  }
}

/** `useStream` for a `stream_json` response: `data` is the parsed JSON once it ends. */
export function useJsonStream<TJsonData = null, TSendBody extends object = {}>(
  url: MaybeRefOrGetter<string>,
  options: Omit<StreamOptions<TSendBody>, 'json'> = {},
) {
  const { data, jsonData, ...rest } = useStream<TSendBody, TJsonData>(url, {
    ...options,
    json: true,
  })
  return { data: jsonData, strData: data, ...rest }
}

/**
 * Listens to an event stream over a GET from mount until its end signal,
 * and connects again when `url` changes.
 */
export function useEventStream(url: MaybeRefOrGetter<string>, options: EventStreamOptions = {}) {
  const state = shallowRef<EventStreamState>({ message: '', messageParts: [] })
  let connection: EventStreamConnection | null = null

  const open = () => {
    connection?.close()
    state.value = { message: '', messageParts: [] }
    connection = openEventStream(
      toValue(url),
      () => options,
      (next) => {
        state.value = next
      },
    )
  }
  let mounted = false
  onMounted(() => {
    mounted = true
    open()
  })
  watch(
    () => toValue(url),
    () => {
      if (mounted) {
        open()
      }
    },
  )
  onBeforeUnmount(() => {
    mounted = false
    connection?.close()
  })

  return {
    message: computed(() => state.value.message),
    messageParts: computed(() => state.value.messageParts),
    close: (resetMessage = false) => connection?.close(resetMessage),
    clearMessage: () => connection?.clear(),
  }
}

/** POSTs a JSON body with `send` and collects the stream's events, each parsed as JSON. */
export function useJsonEventStream<TEvent = unknown, TSendBody extends object = {}>(
  url: MaybeRefOrGetter<string>,
  options: JsonEventStreamOptions<TEvent, TSendBody> = {},
) {
  const stream = new JsonEventStream<TEvent, TSendBody>(() => options)
  const state = shallowRef(stream.snapshot())
  const unsubscribe = stream.subscribe(() => {
    state.value = stream.snapshot()
  })
  const send = (body?: TSendBody) => stream.send(toValue(url), body)

  onMounted(() => {
    window.addEventListener('beforeunload', stream.cancel)
    if (options.initialInput) {
      void send(options.initialInput)
    }
  })
  onBeforeUnmount(() => {
    window.removeEventListener('beforeunload', stream.cancel)
    stream.cancel()
    unsubscribe()
  })

  return {
    events: computed(() => state.value.events),
    isFetching: computed(() => state.value.isFetching),
    isStreaming: computed(() => state.value.isStreaming),
    send,
    cancel: stream.cancel,
    clearEvents: stream.clearEvents,
  }
}
