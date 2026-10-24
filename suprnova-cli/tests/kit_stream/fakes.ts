// Stand-ins for `fetch` and `EventSource` that a kit's stream hook test
// drives by hand: each request waits for `respond`, then its body takes
// whatever the test pushes until `end`.

export type FakeRequest = {
  url: string
  init: RequestInit
  respond: (type?: string, status?: number, body?: string) => void
  push: (text: string) => void
  end: () => void
  aborted: () => boolean
}

export function fakeFetch() {
  const requests: FakeRequest[] = []
  const fetch = (url: string, init: RequestInit): Promise<Response> =>
    new Promise((resolve, reject) => {
      const encoder = new TextEncoder()
      let controller!: ReadableStreamDefaultController<Uint8Array>
      let aborted = false
      const body = new ReadableStream<Uint8Array>({
        start(started) {
          controller = started
        },
      })
      init.signal?.addEventListener('abort', () => {
        aborted = true
        const error = new DOMException('The request was aborted.', 'AbortError')
        reject(error)
        try {
          controller.error(error)
        } catch {
          // Already closed.
        }
      })
      requests.push({
        url,
        init,
        respond: (type = 'text/plain', status = 200, text) =>
          resolve(
            new Response(text ?? body, { status, headers: { 'Content-Type': type } }),
          ),
        push: (text) => controller.enqueue(encoder.encode(text)),
        end: () => controller.close(),
        aborted: () => aborted,
      })
    })
  return { fetch, requests }
}

export class FakeEventSource extends EventTarget {
  static opened: FakeEventSource[] = []
  readonly url: string
  closed = false

  constructor(url: string) {
    super()
    this.url = url
    FakeEventSource.opened.push(this)
  }

  close(): void {
    this.closed = true
  }

  emit(name: string, data: string): void {
    this.dispatchEvent(new MessageEvent(name, { data }))
  }
}

/** Lets pending promises and the timers they queue run. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}
