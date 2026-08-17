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
type LifecycleListener = (event: Pick<FetchEventLike, 'waitUntil'>) => void

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
    cache,
    cacheStorage,
    entries,
    listener: listeners.get('fetch') as FetchListener,
    lifecycleListener(type: 'install' | 'activate') {
      return listeners.get(type) as LifecycleListener
    },
    workerGlobal,
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

describe('public service worker static cache', () => {
  it('stores a successful allowlisted cache miss for later requests', async () => {
    const online = vi.fn(async () => new Response('logo', { status: 200 }))
    const loaded = loadServiceWorker(online as typeof fetch)
    const request = new Request(`${ORIGIN}/logo.png`)

    const first = await dispatchFetch(loaded.listener, request)
    const second = await dispatchFetch(loaded.listener, request)

    expect(await first.text()).toBe('logo')
    expect(await second.text()).toBe('logo')
    expect(online).toHaveBeenCalledTimes(1)
    expect(loaded.cache.put).toHaveBeenCalledTimes(1)
  })

  it('does not cache unsuccessful allowlisted responses', async () => {
    const online = vi.fn(async () => new Response('missing', { status: 404 }))
    const loaded = loadServiceWorker(online as typeof fetch)
    const request = new Request(`${ORIGIN}/logo.png`)

    await dispatchFetch(loaded.listener, request)
    await dispatchFetch(loaded.listener, request)

    expect(online).toHaveBeenCalledTimes(2)
    expect(loaded.cache.put).not.toHaveBeenCalled()
  })

  it('returns the network response when a cache refill fails', async () => {
    const online = vi.fn(async () => new Response('logo', { status: 200 }))
    const loaded = loadServiceWorker(online as typeof fetch)
    loaded.cache.put.mockRejectedValueOnce(new Error('quota exceeded'))

    const response = await dispatchFetch(
      loaded.listener,
      new Request(`${ORIGIN}/logo.png`),
    )

    expect(await response.text()).toBe('logo')
    expect(online).toHaveBeenCalledOnce()
  })
})

describe('public service worker lifecycle', () => {
  it('keeps skipWaiting inside successful install lifetime work', async () => {
    const loaded = loadServiceWorker(vi.fn() as unknown as typeof fetch)
    const lifetimeWork: Promise<unknown>[] = []

    loaded.lifecycleListener('install')({
      waitUntil(work) {
        lifetimeWork.push(work)
      },
    })

    expect(loaded.workerGlobal.skipWaiting).not.toHaveBeenCalled()
    await Promise.all(lifetimeWork)
    expect(loaded.workerGlobal.skipWaiting).toHaveBeenCalledOnce()
  })

  it('does not skip waiting when precaching fails', async () => {
    const loaded = loadServiceWorker(vi.fn() as unknown as typeof fetch)
    loaded.cache.addAll.mockRejectedValueOnce(new Error('cache unavailable'))
    const lifetimeWork: Promise<unknown>[] = []

    loaded.lifecycleListener('install')({
      waitUntil(work) {
        lifetimeWork.push(work)
      },
    })

    await expect(Promise.all(lifetimeWork)).rejects.toThrow('cache unavailable')
    expect(loaded.workerGlobal.skipWaiting).not.toHaveBeenCalled()
  })

  it('claims clients only after old caches are cleaned', async () => {
    const loaded = loadServiceWorker(vi.fn() as unknown as typeof fetch)
    loaded.cacheStorage.keys.mockResolvedValueOnce(['librefang-v2', 'librefang-v3'])
    const lifetimeWork: Promise<unknown>[] = []

    loaded.lifecycleListener('activate')({
      waitUntil(work) {
        lifetimeWork.push(work)
      },
    })

    expect(loaded.workerGlobal.clients.claim).not.toHaveBeenCalled()
    await Promise.all(lifetimeWork)
    expect(loaded.cacheStorage.delete).toHaveBeenCalledWith('librefang-v2')
    expect(loaded.cacheStorage.delete).not.toHaveBeenCalledWith('librefang-v3')
    expect(loaded.workerGlobal.clients.claim).toHaveBeenCalledOnce()
  })
})
