import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { describe, expect, it, vi } from 'vitest'

const ORIGIN = 'https://librefang.ai'
const SERVICE_WORKER_SOURCE = readFileSync(
  new URL('../public/sw.js', import.meta.url),
  'utf8',
)

type FetchEventLike = {
  request: Request
  respondWith(response: Promise<Response | undefined>): void
  waitUntil(work: Promise<unknown>): void
}

type FetchListener = (event: FetchEventLike) => void

function cacheKey(input: RequestInfo | URL): string {
  if (typeof input === 'string') return new URL(input, ORIGIN).href
  if (input instanceof URL) return input.href
  return input.url
}

function loadServiceWorker(fetchImpl: typeof fetch) {
  const entries = new Map<string, Response>()
  const listeners = new Map<string, (event: never) => void>()
  const cache = {
    addAll: vi.fn(async () => undefined),
    match: vi.fn(async (request: RequestInfo | URL) => entries.get(cacheKey(request))),
    put: vi.fn(async (request: RequestInfo | URL, response: Response) => {
      entries.set(cacheKey(request), response)
    }),
  }
  const cacheStorage = {
    open: vi.fn(async () => cache),
    match: vi.fn(async (request: RequestInfo | URL) => entries.get(cacheKey(request))),
    keys: vi.fn(async () => []),
    delete: vi.fn(async () => true),
  }
  const workerGlobal = {
    location: { origin: ORIGIN },
    clients: { claim: vi.fn() },
    skipWaiting: vi.fn(),
    addEventListener: vi.fn((type: string, listener: (event: never) => void) => {
      listeners.set(type, listener)
    }),
  }

  runInNewContext(SERVICE_WORKER_SOURCE, {
    self: workerGlobal,
    caches: cacheStorage,
    fetch: fetchImpl,
    Request,
    Response,
    URL,
  })

  return {
    entries,
    listener: listeners.get('fetch') as FetchListener,
  }
}

async function dispatchFetch(listener: FetchListener, request: Request): Promise<Response> {
  const lifetimeWork: Promise<unknown>[] = []
  let responsePromise: Promise<Response | undefined> | undefined

  listener({
    request,
    respondWith(response) {
      responsePromise = response
    },
    waitUntil(work) {
      lifetimeWork.push(work)
    },
  })

  if (!responsePromise) throw new Error('service worker did not handle the request')
  const response = await responsePromise
  await Promise.all(lifetimeWork)
  if (!response) throw new Error('service worker returned no offline response')
  return response
}

describe('public service worker HTML cache', () => {
  it('refreshes one SPA shell and serves it for an offline deep link', async () => {
    const online = vi.fn(async () => new Response('<html>fresh shell</html>'))
    const loaded = loadServiceWorker(online as typeof fetch)
    const navigation = new Request(`${ORIGIN}/ja/registry?category=hands`, {
      headers: { accept: 'text/html' },
    })

    const networkResponse = await dispatchFetch(loaded.listener, navigation)

    expect(await networkResponse.text()).toBe('<html>fresh shell</html>')
    expect([...loaded.entries.keys()]).toEqual([`${ORIGIN}/`])
    expect(await loaded.entries.get(`${ORIGIN}/`)?.text()).toBe('<html>fresh shell</html>')

    const offline = vi.fn(async () => {
      throw new TypeError('offline')
    })
    const offlineLoaded = loadServiceWorker(offline as typeof fetch)
    offlineLoaded.entries.set(
      `${ORIGIN}/`,
      new Response('<html>latest cached shell</html>'),
    )

    const fallback = await dispatchFetch(
      offlineLoaded.listener,
      new Request(`${ORIGIN}/es/deploy`, { headers: { accept: 'text/html' } }),
    )

    expect(await fallback.text()).toBe('<html>latest cached shell</html>')
  })
})
