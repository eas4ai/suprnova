import { describe, expect, test } from 'bun:test'
import { createStaticClient, readStaticPublication } from './core'
import type { HttpClient, HttpRequestConfig, HttpResponse } from '@inertiajs/core'

const publication = { version: 'build-a', pages: { '/': '/_pub/build-a/index.json', '/about': '/_pub/build-a/about.json' } }
const visit: HttpRequestConfig = { method: 'get', url: 'https://app.test/about', headers: { 'X-Inertia': true, 'X-Inertia-Version': 'build-a' } }
function page(overrides: Record<string, unknown> = {}): HttpResponse {
  return { status: 200, headers: { 'content-type': 'application/json' }, data: JSON.stringify({
    component: 'About', props: { title: 'Public' }, version: 'build-a', url: '/about', ...overrides,
  }) }
}
function fixture(respond: (config: HttpRequestConfig, count: number) => Promise<HttpResponse> = async () => page()) {
  const requests: HttpRequestConfig[] = []
  let cookie = 'XSRF-TOKEN=first'
  const fetches: Array<[string, RequestInit | undefined]> = []
  const inner: HttpClient = { async request(config) { requests.push(config); return respond(config, requests.length) } }
  const environment = {
    href: 'https://app.test/', cookie: () => cookie,
    fetch: (async (url: string | URL | Request, init?: RequestInit) => {
      fetches.push([String(url), init]); cookie = 'XSRF-TOKEN=fresh%20token'; return new Response('', { status: 200 })
    }) as typeof fetch,
  }
  return { requests, inner, environment, fetches, setCookie: (value: string) => { cookie = value },
    client: createStaticClient(inner, publication, environment) }
}

describe('PUB-005 shared transport', () => {
  test('public navigation fetches the versioned object and keeps the canonical visit', async () => {
    const f = fixture(async () => page({ url: 'https://app.test/_pub/build-a/about.json' }))
    const config = { ...visit, url: 'https://app.test/about#team' }
    const result = await f.client.request(config)
    expect(f.requests[0].url).toBe('https://app.test/_pub/build-a/about.json')
    expect(JSON.parse(result.data).url).toBe('/about#team')
    expect(result.headers['x-inertia']).toBe('true')
    expect(config.url).toBe('https://app.test/about#team')
    expect(f.fetches).toHaveLength(0)
    await f.client.request({ ...visit, url: '/' })
    expect(f.requests[1].url).toBe('https://app.test/_pub/build-a/index.json')
  })

  test('metadata absence leaves the original transport unchanged', () => {
    const f = fixture()
    expect(createStaticClient(f.inner, null, f.environment)).toBe(f.inner)
    expect(readStaticPublication({ querySelector: () => null, getElementById: () => null } as unknown as Document)).toBeNull()
  })

  test('metadata reads the document version and concrete path map', () => {
    const doc = { querySelector: () => ({ content: publication.version }), getElementById: () => ({ textContent: JSON.stringify(publication.pages) }) }
    expect(readStaticPublication(doc as unknown as Document)).toEqual(publication)
    expect(() => readStaticPublication({ ...doc, getElementById: () => ({ textContent: '[]' }) } as unknown as Document)).toThrow('page map')
  })

  for (const [reason, headers] of Object.entries({
    partial: { 'X-Inertia-Partial-Data': 'title' },
    except: { 'x-inertia-partial-except': 'title' },
    deferred: { 'X-Inertia-Partial-Component': 'About', 'X-Inertia-Partial-Data': 'deferred' },
    once: { 'X-Inertia-Except-Once-Props': 'countries' },
    polling: { 'Cache-Control': 'no-cache' },
    reload: { 'cache-control': 'no-store' },
    reset: { 'X-Inertia-Reset': 'items' },
    infinite: { 'X-Inertia-Infinite-Scroll-Merge-Intent': 'append' },
    bypass: { 'X-Inertia-Static-Bypass': '1' },
  })) {
    test(`${reason} uses the private origin and no-store`, async () => {
      const f = fixture()
      await f.client.request({ ...visit, headers: { ...visit.headers, ...headers } })
      expect(f.requests[0].url).toBe('https://app.test/about?__suprnova_origin=1')
      expect(f.requests[0].headers?.['Cache-Control']).toBe('no-store')
    })
  }

  test('private query keeps filters and strips the transport marker from absolute page.url', async () => {
    const f = fixture(async () => page({ url: 'https://app.test/about?search=hello%20world&__suprnova_origin=1' }))
    const result = await f.client.request({ ...visit, url: '/about?search=hello%20world' })
    const requested = new URL(f.requests[0].url)
    expect(requested.searchParams.get('search')).toBe('hello world')
    expect(requested.searchParams.get('__suprnova_origin')).toBe('1')
    expect(JSON.parse(result.data).url).toBe('https://app.test/about?search=hello+world')
  })

  test('query params and GET data go to the private origin', async () => {
    const f = fixture()
    for (const change of [{ params: { search: 'hello' } }, { data: { filter: 'new' } }]) {
      await f.client.request({ ...visit, ...change })
    }
    expect(f.requests.every((config) => new URL(config.url).searchParams.get('__suprnova_origin') === '1')).toBe(true)
    expect(f.requests[0].params).toEqual({ search: 'hello' })
  })

  test('unknown full GET reaches the canonical origin for first-visit publication', async () => {
    const f = fixture(async () => page({ url: '/new-public-page', flash: { message: 'Origin only' } }))
    const response = await f.client.request({ ...visit, url: '/new-public-page' })
    expect(f.requests).toHaveLength(1)
    expect(f.requests[0].url).toBe('/new-public-page')
    expect(f.requests[0].headers?.['Cache-Control']).toBeUndefined()
    expect(JSON.parse(response.data).flash.message).toBe('Origin only')
    await f.client.request({ ...visit, url: '/new-public-page', headers: { ...visit.headers, 'X-Inertia-Partial-Data': 'private' } })
    expect(f.requests[1].url).toBe('https://app.test/new-public-page?__suprnova_origin=1')
  })

  for (const version of ['build-b', null]) {
    test(`version mismatch ${version} reloads the canonical document`, async () => {
      const f = fixture(async () => page({ version }))
      const response = await f.client.request(visit)
      expect(response).toEqual({ status: 409, data: '', headers: { 'x-inertia-location': 'https://app.test/about' } })
      expect(f.requests).toHaveLength(1)
    })
  }

  test('a stale request version reloads even when the object matches document metadata', async () => {
    const f = fixture()
    expect((await f.client.request({ ...visit, headers: { ...visit.headers, 'X-Inertia-Version': 'old' } })).status).toBe(409)
  })

  for (const privatePage of [
    { flash: { success: 'Private message' } },
    { props: { flash: { message: 'Private message' } } },
    { props: { errors: { email: 'Private validation' } } },
  ]) {
    test(`private props ${JSON.stringify(privatePage)} are accepted only from origin`, async () => {
      const f = fixture(async (_config, count) => count === 1 ? page(privatePage) : page({ props: { errors: { name: 'From origin' } }, flash: { success: 'From origin' } }))
      const response = await f.client.request(visit)
      expect(f.requests).toHaveLength(2)
      expect(f.requests[1].url).toBe('https://app.test/about?__suprnova_origin=1')
      expect(JSON.parse(response.data).flash.success).toBe('From origin')
    })
  }

  test('missing or malformed objects fall back once to the private origin', async () => {
    for (const missing of [403, 404, 410, 200]) {
      const f = fixture(async (_config, count) => {
        if (count > 1) return page()
        if (missing === 200) return { status: 200, data: '<html>Fallback</html>', headers: {} }
        throw { response: { status: missing } }
      })
      await f.client.request(visit)
      expect(f.requests).toHaveLength(2)
      expect(f.requests[1].headers?.['Cache-Control']).toBe('no-store')
    }
  })

  test('network failures propagate without a hidden retry', async () => {
    const failure = new Error('network failed')
    const f = fixture(async () => { throw failure })
    await expect(f.client.request(visit)).rejects.toBe(failure)
    expect(f.requests).toHaveLength(1)
  })

  test('mutations share a private CSRF cookie bootstrap before XHR reads the current token', async () => {
    const f = fixture(async () => {
      expect(f.environment.cookie()).toBe('XSRF-TOKEN=fresh%20token')
      return page()
    })
    f.setCookie('')
    await Promise.all([f.client.request({ ...visit, method: 'post' }), f.client.request({ ...visit, method: 'delete' })])
    expect(f.fetches).toHaveLength(1)
    expect(f.fetches[0][0]).toBe('https://app.test/?__suprnova_origin=1')
    expect(f.fetches[0][1]?.credentials).toBe('same-origin')
    expect(f.fetches[0][1]?.cache).toBe('no-store')
    expect(f.requests).toHaveLength(2)
    expect(f.requests.every((config) => new URL(config.url).searchParams.get('__suprnova_origin') === '1')).toBe(true)
  })

  test('a cookie bootstrap refusal sends no mutation', async () => {
    const f = fixture()
    f.setCookie('')
    f.environment.fetch = (async () => new Response('', { status: 503 })) as typeof fetch
    const client = createStaticClient(f.inner, publication, f.environment)
    await expect(client.request({ ...visit, method: 'post' })).rejects.toThrow('503')
    expect(f.requests).toHaveLength(0)
  })

  test('origin must issue the cookie before a mutation', async () => {
    const f = fixture()
    f.setCookie('')
    f.environment.fetch = (async () => new Response('', { status: 200 })) as typeof fetch
    const client = createStaticClient(f.inner, publication, f.environment)
    await expect(client.request({ ...visit, method: 'post' })).rejects.toThrow('did not issue')
    expect(f.requests).toHaveLength(0)
  })

  test('cross-origin, non-Inertia and malicious object targets never receive a transport rewrite', async () => {
    const f = fixture()
    await f.client.request({ ...visit, method: 'post', url: 'https://other.test/about' })
    await f.client.request({ ...visit, headers: {} })
    expect(f.requests[0].url).toBe('https://other.test/about')
    expect(f.requests[1].url).toBe(visit.url)
    expect(f.fetches).toHaveLength(0)
    const client = createStaticClient(f.inner, { ...publication, pages: { '/about': 'https://other.test/private' } }, f.environment)
    await client.request(visit)
    expect(f.requests[2].url).toBe('https://app.test/about?__suprnova_origin=1')
  })
})
