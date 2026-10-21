// The client side of the responses Suprnova streams. `HttpResponse::stream_bytes`
// and `stream_json` are read by `useStream` and `useJsonStream`;
// `HttpResponse::event_stream` is read by `useEventStream` over a GET, or by
// `useJsonEventStream` after a request that sends a body.
//
// The hooks in `index.ts` keep their state here, so this file imports no UI
// framework. It touches the browser only when a stream starts, never at
// import, so the server render can import it.

/** Where a stream request finds the session's CSRF token. */
export type CsrfHeaderOptions = {
  /** The token itself, when the page has it from somewhere else. */
  csrfToken?: string
  /** The cookie `CsrfMiddleware` writes the token to: `XSRF-TOKEN`. */
  xsrfCookieName?: string
  /** The header the cookie's value goes back in: `X-XSRF-TOKEN`. */
  xsrfHeaderName?: string
}

/**
 * The header that carries the CSRF token: the token given, else the
 * configured XSRF cookie, else the `csrf-token` meta tag. The cookie wins
 * because sign-in rotates it while Inertia can leave the meta tag stale.
 * Empty outside the browser unless a token is given.
 */
export function csrfHeaders({
  csrfToken,
  xsrfCookieName = 'XSRF-TOKEN',
  xsrfHeaderName = 'X-XSRF-TOKEN',
}: CsrfHeaderOptions = {}): Record<string, string> {
  if (csrfToken) {
    return { 'X-CSRF-TOKEN': csrfToken }
  }
  if (typeof document === 'undefined') {
    return {}
  }
  const prefix = `${xsrfCookieName}=`
  for (const entry of document.cookie.split(';')) {
    const cookie = entry.trim()
    if (cookie.startsWith(prefix)) {
      return { [xsrfHeaderName]: decodeURIComponent(cookie.slice(prefix.length)) }
    }
  }
  const meta = document.querySelector('meta[name="csrf-token"]')?.getAttribute('content')
  return meta ? { 'X-CSRF-TOKEN': meta } : {}
}

/** One server-sent event that carried data. */
export type StreamFrame = {
  /** The event's name, or `null` when it named none (a `message`). */
  event: string | null
  /** The stream's last event id, which holds until the server sends another. */
  id: string | null
  /** The event's data lines, joined with a line break. */
  data: string
}

/**
 * Reads `body` as server-sent events, calling `onFrame` with each event
 * that carries data until it returns `false` or the body ends. An event
 * the body ends inside, before its blank line, is dropped, as the
 * specification says.
 */
export async function readEventStream(
  body: ReadableStream<Uint8Array>,
  onFrame: (frame: StreamFrame) => boolean,
): Promise<void> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let id: string | null = null
  let event: string | null = null
  let data: string[] | null = null

  // One line of the stream; false stops reading.
  const line = (text: string): boolean => {
    if (text === '') {
      const frame = data === null ? null : { event, id, data: data.join('\n') }
      event = null
      data = null
      return frame === null || onFrame(frame)
    }
    if (text.startsWith(':')) {
      return true
    }
    const colon = text.indexOf(':')
    const field = colon === -1 ? text : text.slice(0, colon)
    let value = colon === -1 ? '' : text.slice(colon + 1)
    if (value.startsWith(' ')) {
      value = value.slice(1)
    }
    if (field === 'data') {
      ;(data ??= []).push(value)
    } else if (field === 'event') {
      event = value === '' ? null : value
    } else if (field === 'id' && !value.includes('\u0000')) {
      id = value
    }
    return true
  }

  try {
    for (;;) {
      const { done, value } = await reader.read()
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true })
      // A carriage return at the end may be the first half of a CRLF that
      // the next read completes; it waits for that read.
      const complete = done || !buffer.endsWith('\r') ? buffer : buffer.slice(0, -1)
      const lines = complete.split(/\r\n|\r|\n/)
      buffer = (lines.pop() ?? '') + buffer.slice(complete.length)
      for (const text of lines) {
        if (!line(text)) {
          void reader.cancel().catch(() => undefined)
          return
        }
      }
      if (done) {
        return
      }
    }
  } finally {
    reader.releaseLock()
  }
}

/** A stream request the server answered with an error status. */
export class StreamResponseError extends Error {
  /** The server's response. */
  readonly response: Response
  /** The response body, such as a JSON validation error. */
  readonly body: string

  constructor(response: Response, body: string) {
    super(body || `The stream request failed with status ${response.status}.`)
    this.name = 'StreamResponseError'
    this.response = response
    this.body = body
  }

  /** The response's status code: 419 for a missing CSRF token, 422 for invalid input. */
  get status(): number {
    return this.response.status
  }
}

/** Changes the request before it is sent, or refuses it with `false`. */
export type BeforeSend = (request: RequestInit) => RequestInit | boolean | void

/** What `streamJsonEvents` sends and the callbacks it reports to. */
export type JsonEventStreamRequest<TEvent = unknown, TSendBody extends object = {}> =
  CsrfHeaderOptions & {
    url: string
    /** `POST` unless given; a `GET` or `HEAD` sends no body. */
    method?: string
    body?: TSendBody
    signal: AbortSignal
    headers?: Record<string, string>
    credentials?: RequestCredentials
    /** Only events with this name, or one of these; every event when unset. */
    eventName?: string | string[]
    /** The data that ends the stream: `</stream>`, what `EndSignal::default()` sends. */
    endSignal?: string
    onEvent: (event: TEvent, frame: StreamFrame) => void
    onSend?: () => void
    onResponse?: (response: Response) => void
    /** An event whose data is not JSON; the stream goes on. */
    onParseError?: (error: Error, data: string) => void
    onBeforeSend?: BeforeSend
  }

/**
 * Sends a JSON body and reads the event stream the server answers with,
 * parsing each event's data as JSON. Resolves `false` when `onBeforeSend`
 * refused the request and `true` when the stream ended; rejects with a
 * `StreamResponseError` for an error status and with an `AbortError` when
 * `signal` aborted it.
 */
export async function streamJsonEvents<TEvent = unknown, TSendBody extends object = {}>(
  request: JsonEventStreamRequest<TEvent, TSendBody>,
): Promise<boolean> {
  const { url, signal, endSignal = '</stream>' } = request
  const method = request.method ?? 'POST'
  const carriesBody = !['GET', 'HEAD'].includes(method.toUpperCase())
  const init: RequestInit = {
    method,
    signal,
    credentials: request.credentials ?? 'same-origin',
    headers: {
      ...(carriesBody ? { 'Content-Type': 'application/json' } : {}),
      Accept: 'text/event-stream',
      ...csrfHeaders(request),
      ...request.headers,
    },
    ...(carriesBody ? { body: JSON.stringify(request.body ?? {}) } : {}),
  }
  const changed = request.onBeforeSend?.(init)
  if (changed === false) {
    return false
  }
  request.onSend?.()
  const response = await fetch(url, typeof changed === 'object' && changed !== null ? changed : init)
  if (!response.ok) {
    throw new StreamResponseError(response, await response.text())
  }
  if (!response.body) {
    throw new Error('This browser cannot read a response as a stream.')
  }
  const type = response.headers.get('Content-Type') ?? ''
  if (!type.includes('text/event-stream')) {
    throw new Error(`Expected a text/event-stream response, received "${type}".`)
  }
  request.onResponse?.(response)
  const names = request.eventName === undefined ? null : [request.eventName].flat()
  await readEventStream(response.body, (frame) => {
    if (signal.aborted) {
      return false
    }
    // Before the name filter, so an end signal under any name ends the stream.
    if (frame.data === endSignal) {
      return false
    }
    if (names !== null && !names.includes(frame.event ?? 'message')) {
      return true
    }
    let event: TEvent
    try {
      event = JSON.parse(frame.data) as TEvent
    } catch (error) {
      request.onParseError?.(error as Error, frame.data)
      return true
    }
    // Outside the try, so an error the callback throws is not taken for bad JSON.
    request.onEvent(event, frame)
    return true
  })
  if (signal.aborted) {
    throw new DOMException('The stream was cancelled.', 'AbortError')
  }
  return true
}

/** What `useStream` and `useJsonStream` take. */
export type StreamOptions<TSendBody extends object = {}> = CsrfHeaderOptions & {
  /** Streams with the same id share their data and callbacks. Random when unset. */
  id?: string
  /** A body to send as soon as the component mounts. */
  initialInput?: TSendBody
  headers?: Record<string, string>
  /** Parse the whole response as JSON when it ends, into `jsonData`. */
  json?: boolean
  credentials?: RequestCredentials
  onResponse?: (response: Response) => void
  /** Each piece of text as it arrives. */
  onData?: (data: string) => void
  onCancel?: () => void
  onFinish?: () => void
  onError?: (error: Error) => void
  onBeforeSend?: BeforeSend
}

/** A text stream's state, which every component reading the stream sees. */
export type StreamState<TJsonData = null> = {
  /** The text received so far. */
  data: string
  /** The whole response parsed as JSON, once it ended, with `json` set. */
  jsonData: TJsonData | null
  /** The request is sent and no response has arrived. */
  isFetching: boolean
  /** The response is arriving. */
  isStreaming: boolean
}

type StreamCallbacks = Pick<
  StreamOptions,
  'onResponse' | 'onData' | 'onCancel' | 'onFinish' | 'onError' | 'onBeforeSend'
>

/** A random id for a stream, sent as `X-STREAM-ID`. */
export function streamId(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(8)), (byte) =>
    byte.toString(16).padStart(2, '0'),
  ).join('')
}

const textStreams = new Map<string, TextStream<unknown>>()

/**
 * The text stream with `id`: the one the components reading it share,
 * or a new one. Browser acquisition registers it immediately, so readers
 * rendered together share one engine before either subscribes. Its last
 * reader cancels and removes it. Without `document`, acquisition never
 * registers an engine, so server renders keep no shared state.
 */
export function textStream<TJsonData = null>(id: string): TextStream<TJsonData> {
  const existing = textStreams.get(id) as TextStream<TJsonData> | undefined
  if (existing) {
    return existing
  }
  const stream = new TextStream<TJsonData>(id)
  if (typeof document !== 'undefined') {
    textStreams.set(id, stream as TextStream<unknown>)
  }
  return stream
}

/** A POST whose response is read as text: `useStream`'s engine. */
export class TextStream<TJsonData = null> {
  readonly id: string
  private state: StreamState<TJsonData> = {
    data: '',
    jsonData: null,
    isFetching: false,
    isStreaming: false,
  }
  private controller: AbortController | null = null
  private readonly listeners = new Set<() => void>()
  private readonly callbacks = new Set<StreamCallbacks>()

  constructor(id: string) {
    this.id = id
  }

  /** The current state; a new object after every change. */
  snapshot = (): StreamState<TJsonData> => this.state

  /** Calls `listener` after every change, until the returned function is called. */
  subscribe = (listener: () => void): (() => void) => {
    if (!textStreams.has(this.id)) {
      textStreams.set(this.id, this as TextStream<unknown>)
    }
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
      if (this.listeners.size === 0) {
        this.cancel()
        if (textStreams.get(this.id) === this) {
          textStreams.delete(this.id)
        }
      }
    }
  }

  /** Reports to `callbacks` until the returned function is called. */
  listen = (callbacks: StreamCallbacks): (() => void) => {
    this.callbacks.add(callbacks)
    return () => {
      this.callbacks.delete(callbacks)
    }
  }

  /**
   * Cancels a request in flight, clears the data and POSTs `body` as JSON
   * to `url`, with the `X-STREAM-ID` and CSRF headers. Resolves when the
   * response has ended, failed or been cancelled. Callbacks may cancel or
   * replace the request, so only its current controller may publish state.
   */
  send = async (url: string, body: object | undefined, options: StreamOptions<object>): Promise<void> => {
    this.cancel()
    if (this.controller !== null) {
      return
    }
    const controller = new AbortController()
    const request: RequestInit = {
      method: 'POST',
      signal: controller.signal,
      credentials: options.credentials ?? 'same-origin',
      headers: {
        'Content-Type': 'application/json',
        'X-STREAM-ID': this.id,
        ...csrfHeaders(options),
        ...options.headers,
      },
      body: JSON.stringify(body ?? {}),
    }
    this.controller = controller
    const current = () => this.controller === controller
    try {
      let changed: RequestInit | null = null
      for (const callbacks of this.callbacks) {
        const result = callbacks.onBeforeSend?.(request)
        if (!current()) {
          return
        }
        if (result === false) {
          this.controller = null
          return
        }
        if (typeof result === 'object' && result !== null) {
          changed = result
          break
        }
      }
      this.set({ data: '', jsonData: null, isFetching: true, isStreaming: false })
      if (!current()) {
        return
      }
      const response = await fetch(url, changed ?? request)
      if (!response.ok) {
        throw new StreamResponseError(response, await response.text())
      }
      if (!response.body) {
        throw new Error('This browser cannot read a response as a stream.')
      }
      if (!current()) {
        return
      }
      this.each((callbacks) => callbacks.onResponse?.(response), current)
      if (!current()) {
        return
      }
      this.set({ isFetching: false, isStreaming: true })
      if (!current()) {
        return
      }
      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      let data = ''
      for (;;) {
        const { done, value } = await reader.read()
        if (!current()) {
          return
        }
        // One decoder for the whole body, so a character split across two
        // reads arrives whole.
        const text = done ? decoder.decode() : decoder.decode(value, { stream: true })
        if (text !== '') {
          data += text
          this.each((callbacks) => callbacks.onData?.(text), current)
          if (!current()) {
            return
          }
        }
        if (done) {
          break
        }
        this.set({ data })
      }
      let jsonData: TJsonData | null = null
      if (options.json) {
        try {
          jsonData = JSON.parse(data) as TJsonData
        } catch (error) {
          this.each((callbacks) => callbacks.onError?.(error as Error), current)
          if (!current()) {
            return
          }
        }
      }
      this.controller = null
      this.set({ data, jsonData, isStreaming: false })
      this.each((callbacks) => callbacks.onFinish?.())
    } catch (error) {
      if (!current()) {
        return
      }
      this.controller = null
      this.set({ isFetching: false, isStreaming: false })
      this.each((callbacks) => callbacks.onError?.(error as Error))
      this.each((callbacks) => callbacks.onFinish?.())
    }
  }

  /** Stops the request in flight, if any, and reports `onCancel`. */
  cancel = (): void => {
    const controller = this.controller
    if (controller === null) {
      return
    }
    this.controller = null
    controller.abort()
    this.set({ isFetching: false, isStreaming: false })
    this.each((callbacks) => callbacks.onCancel?.())
  }

  /** Empties `data` and `jsonData`. */
  clearData = (): void => {
    this.set({ data: '', jsonData: null })
  }

  private set(change: Partial<StreamState<TJsonData>>): void {
    this.state = { ...this.state, ...change }
    for (const listener of this.listeners) {
      listener()
    }
  }

  private each(report: (callbacks: StreamCallbacks) => void, current?: () => boolean): void {
    for (const callbacks of this.callbacks) {
      if (current && !current()) {
        return
      }
      report(callbacks)
    }
  }
}

/** What `useEventStream` takes. */
export type EventStreamOptions = {
  /** The event names to listen for: `update`, what `StreamedEvent::message` sends. */
  eventName?: string | string[]
  /** The data that closes the stream: `</stream>`. */
  endSignal?: string
  /** What joins the messages in `message`: a space. */
  glue?: string
  /** Keep only the latest message instead of joining them. */
  replace?: boolean
  onMessage?: (event: MessageEvent) => void
  onComplete?: () => void
  onError?: (error: Event) => void
}

/** The messages an event stream has delivered. */
export type EventStreamState = {
  /** The messages joined with `glue`. */
  message: string
  /** Each message. */
  messageParts: string[]
}

/** An open `EventSource`: `useEventStream`'s engine. */
export type EventStreamConnection = {
  /** Closes the connection; with `true`, also clears the messages. */
  close: (resetMessage?: boolean) => void
  /** Clears the messages and keeps the connection. */
  clear: () => void
}

/**
 * Opens an `EventSource` on `url` and reports its messages to `onChange`
 * until the end signal arrives or the caller closes it. Errors are reported
 * without closing the source, so the browser can reconnect after an interruption.
 * Reads the options each time it uses them, so a component can hand it
 * callbacks that change between renders.
 */
export function openEventStream(
  url: string,
  options: () => EventStreamOptions,
  onChange: (state: EventStreamState) => void,
): EventStreamConnection {
  let parts: string[] = []
  const names = [options().eventName ?? 'update'].flat()
  const source = new EventSource(url)

  const publish = () => {
    onChange({ message: parts.join(options().glue ?? ' '), messageParts: [...parts] })
  }
  const clear = () => {
    parts = []
    publish()
  }
  const message = (event: Event) => {
    const received = event as MessageEvent<string>
    const current = options()
    if (received.data === (current.endSignal ?? '</stream>')) {
      close()
      current.onComplete?.()
      return
    }
    parts = current.replace ? [received.data] : [...parts, received.data]
    publish()
    current.onMessage?.(received)
  }
  const error = (event: Event) => {
    options().onError?.(event)
  }
  const close = (resetMessage = false) => {
    for (const name of names) {
      source.removeEventListener(name, message)
    }
    source.removeEventListener('error', error)
    source.close()
    if (resetMessage) {
      clear()
    }
  }

  for (const name of names) {
    source.addEventListener(name, message)
  }
  source.addEventListener('error', error)
  return { close, clear }
}

/** What `useJsonEventStream` takes. */
export type JsonEventStreamOptions<TEvent = unknown, TSendBody extends object = {}> =
  CsrfHeaderOptions & {
    headers?: Record<string, string>
    credentials?: RequestCredentials
    eventName?: string | string[]
    endSignal?: string
    /** A body to send as soon as the component mounts. */
    initialInput?: TSendBody
    onEvent?: (event: TEvent) => void
    onResponse?: (response: Response) => void
    onParseError?: (error: Error, data: string) => void
    onCancel?: () => void
    onFinish?: () => void
    onError?: (error: Error) => void
    onBeforeSend?: BeforeSend
  }

/** The events a JSON event stream has delivered. */
export type JsonEventStreamState<TEvent = unknown> = {
  events: TEvent[]
  isFetching: boolean
  isStreaming: boolean
}

/** A request whose event stream is read as JSON: `useJsonEventStream`'s engine. */
export class JsonEventStream<TEvent = unknown, TSendBody extends object = {}> {
  private state: JsonEventStreamState<TEvent> = { events: [], isFetching: false, isStreaming: false }
  private controller: AbortController | null = null
  private readonly listeners = new Set<() => void>()
  private readonly options: () => JsonEventStreamOptions<TEvent, TSendBody>

  /** Reads the options each time it sends, so they may change between renders. */
  constructor(options: () => JsonEventStreamOptions<TEvent, TSendBody>) {
    this.options = options
  }

  /** The current state; a new object after every change. */
  snapshot = (): JsonEventStreamState<TEvent> => this.state

  /** Calls `listener` after every change, until the returned function is called. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  /**
   * Cancels a request in flight, then POSTs `body` as JSON to `url` and
   * collects the events of the stream the server answers with. Resolves
   * when the stream has ended, failed or been cancelled.
   */
  send = async (url: string, body?: TSendBody): Promise<void> => {
    const options = this.options()
    this.cancel()
    const controller = new AbortController()
    const finish = () => {
      this.controller = null
      this.set({ isFetching: false, isStreaming: false })
      options.onFinish?.()
    }
    try {
      const sent = await streamJsonEvents<TEvent, TSendBody>({
        ...options,
        url,
        body,
        signal: controller.signal,
        onSend: () => {
          this.controller = controller
          this.set({ events: [], isFetching: true, isStreaming: false })
        },
        onResponse: (response) => {
          this.set({ isFetching: false, isStreaming: true })
          options.onResponse?.(response)
        },
        onEvent: (event) => {
          this.set({ events: [...this.state.events, event] })
          options.onEvent?.(event)
        },
      })
      if (sent) {
        finish()
      }
    } catch (error) {
      if (controller.signal.aborted) {
        return
      }
      this.controller = null
      this.set({ isFetching: false, isStreaming: false })
      options.onError?.(error as Error)
      finish()
    }
  }

  /** Stops the request in flight, if any, and reports `onCancel`. */
  cancel = (): void => {
    const controller = this.controller
    if (controller === null) {
      return
    }
    this.controller = null
    controller.abort()
    this.set({ isFetching: false, isStreaming: false })
    this.options().onCancel?.()
  }

  /** Empties `events`. */
  clearEvents = (): void => {
    this.set({ events: [] })
  }

  private set(change: Partial<JsonEventStreamState<TEvent>>): void {
    this.state = { ...this.state, ...change }
    for (const listener of this.listeners) {
      listener()
    }
  }
}
