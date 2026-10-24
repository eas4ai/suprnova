import type { HttpClient, HttpRequestConfig, HttpResponse } from '@inertiajs/core'

export interface StaticPublication {
  version: string
  pages: Record<string, string>
}

export interface StaticEnvironment {
  href: string
  cookie: () => string
  fetch: typeof fetch
}

const originMarker = '__suprnova_origin'

function header(headers: HttpRequestConfig['headers'], name: string): unknown {
  const key = Object.keys(headers ?? {}).find((key) => key.toLowerCase() === name.toLowerCase())
  return key === undefined ? undefined : headers?.[key]
}

/** Missing metadata keeps development and applications without publication unchanged. */
export function readStaticPublication(doc: Document): StaticPublication | null {
  const version = doc.querySelector<HTMLMetaElement>('meta[name="suprnova-static-version"]')?.content
  const source = doc.getElementById('suprnova-static-pages')?.textContent
  if (!version || !source) return null
  const pages: unknown = JSON.parse(source)
  if (!pages || typeof pages !== 'object' || Array.isArray(pages) ||
      Object.values(pages).some((path) => typeof path !== 'string')) {
    throw new Error('The static publication page map must contain JSON object paths')
  }
  return { version, pages: pages as Record<string, string> }
}

function privateUrl(url: URL): string {
  const privateUrl = new URL(url)
  privateUrl.searchParams.set(originMarker, '1')
  privateUrl.hash = ''
  return privateUrl.href
}

function needsOrigin(config: HttpRequestConfig, url: URL): boolean {
  if (url.search || (config.params && Object.keys(config.params).length > 0) ||
      (config.data && (typeof config.data !== 'object' || Object.keys(config.data).length > 0))) return true
  return Object.entries(config.headers ?? {}).some(([name, value]) => {
    if (value === undefined || value === null || value === false || value === '') return false
    const lower = name.toLowerCase()
    return lower.startsWith('x-inertia-partial-') || lower.includes('once-props') ||
      ['x-inertia-reset', 'x-inertia-infinite-scroll-merge-intent',
        'x-inertia-static-bypass', 'precognition'].includes(lower) ||
      (lower === 'cache-control' && /no-cache|no-store/i.test(String(value)))
  })
}

function pageData(response: HttpResponse): Record<string, unknown> | null {
  try {
    const page: unknown = JSON.parse(response.data)
    if (!page || typeof page !== 'object') return null
    const data = page as Record<string, unknown>
    return typeof data.component === 'string' && typeof data.url === 'string' &&
      data.props !== null && typeof data.props === 'object' && !Array.isArray(data.props) &&
      'version' in data ? data : null
  } catch {
    return null
  }
}

function hasPrivateValue(value: unknown): boolean {
  if (value === null || value === undefined) return false
  return typeof value !== 'object' || Object.keys(value).length > 0
}

function reload(url: URL): HttpResponse {
  url.searchParams.delete(originMarker)
  return { status: 409, data: '', headers: { 'x-inertia-location': url.href } }
}

/** Decorate the transport only. Inertia retains the original visit and history URL. */
export function createStaticClient(
  inner: HttpClient,
  publication: StaticPublication | null,
  environment?: StaticEnvironment,
): HttpClient {
  if (!publication) return inner
  const browser = environment ?? {
    href: window.location.href,
    cookie: () => document.cookie,
    fetch: window.fetch.bind(window),
  }
  let csrfRequest: Promise<void> | null = null

  async function ensureCsrf(config: HttpRequestConfig, url: URL): Promise<void> {
    if (['get', 'head', 'options'].includes(config.method.toLowerCase()) ||
        browser.cookie().split(';').some((cookie) => /^\s*XSRF-TOKEN=.+/.test(cookie))) return
    csrfRequest ??= browser.fetch(privateUrl(new URL(browser.href)), {
      credentials: 'same-origin', cache: 'no-store', headers: { 'Cache-Control': 'no-store' },
    }).then((response) => {
      if (!response.ok) throw new Error(`The CSRF cookie request failed (${response.status})`)
    }).finally(() => { csrfRequest = null })
    await csrfRequest
    if (!browser.cookie().split(';').some((cookie) => /^\s*XSRF-TOKEN=.+/.test(cookie))) {
      throw new Error(`The origin did not issue an XSRF-TOKEN cookie for ${url.pathname}`)
    }
  }

  async function origin(config: HttpRequestConfig, url: URL): Promise<HttpResponse> {
    const response = await inner.request({ ...config, url: privateUrl(url),
      headers: { ...config.headers, 'Cache-Control': 'no-store' } })
    const page = pageData(response)
    if (!page) return response
    const canonical = new URL(String(page.url), browser.href)
    canonical.searchParams.delete(originMarker)
    page.url = String(page.url).startsWith('/') ? canonical.pathname + canonical.search + canonical.hash : canonical.href
    return { ...response, data: JSON.stringify(page) }
  }

  return {
    async request(config) {
      const url = new URL(config.url, browser.href)
      if (url.origin !== new URL(browser.href).origin || !header(config.headers, 'X-Inertia')) {
        return inner.request(config)
      }
      await ensureCsrf(config, url)
      const path = publication.pages[url.pathname]
      if (config.method.toLowerCase() !== 'get' || needsOrigin(config, url)) return origin(config, url)
      // An unknown public path must reach the origin's first-visit publisher.
      if (!path) return inner.request(config)
      const object = new URL(path, url.origin + '/')
      // Only the document's own origin can receive Inertia and XSRF headers.
      if (object.origin !== url.origin || object.search || object.hash) return origin(config, url)
      let response: HttpResponse
      try {
        response = await inner.request({ ...config, url: object.href })
      } catch (error) {
        const status = (error as { response?: { status?: number } }).response?.status
        if (status !== undefined && [403, 404, 410].includes(status)) return origin(config, url)
        throw error
      }
      const page = pageData(response)
      if (response.status !== 200 || !page) return origin(config, url)
      if (page.version !== publication.version ||
          (header(config.headers, 'X-Inertia-Version') !== undefined &&
            page.version !== header(config.headers, 'X-Inertia-Version'))) return reload(url)
      const props = page.props as Record<string, unknown>
      if (hasPrivateValue(page.flash) || hasPrivateValue(props.flash) || hasPrivateValue(props.errors)) return origin(config, url)
      page.url = url.pathname + url.search + url.hash
      return { ...response, data: JSON.stringify(page), headers: { ...response.headers, 'x-inertia': 'true' } }
    },
  }
}
