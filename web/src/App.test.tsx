// @vitest-environment jsdom
// @vitest-environment-options { "url": "https://librefang.local/" }

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { getTranslation, type Translation } from './i18n'

const withStats = {
  common: {
    contributors: 'Contributors',
    viewFull: 'View full',
  },
  githubStats: {
    label: 'Community',
    title: 'GitHub stats',
    desc: 'Project activity',
    stars: 'Stars',
    forks: 'Forks',
    issues: 'Issues',
    prs: 'Pull requests',
    downloads: 'Downloads',
    docsVisits: 'Docs visits',
    lastUpdate: 'Last update',
    starHistory: 'Star history',
    starUs: 'Star us',
    discuss: 'Discuss',
  },
  community: {
    label: 'Community',
    title: 'Join us',
    desc: 'Project links',
    items: Array.from({ length: 4 }, (_, index) => ({
      label: `Link ${index}`,
      desc: 'Description',
    })),
    open: 'Open',
  },
} as unknown as Translation

describe('GitHubStats', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('keeps hook order stable and aborts fetches on unmount', async () => {
    const actEnvironment = globalThis as typeof globalThis & {
      IS_REACT_ACT_ENVIRONMENT: boolean
    }
    actEnvironment.IS_REACT_ACT_ENVIRONMENT = true
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    vi.stubGlobal('IntersectionObserver', class {
      readonly root = null
      readonly rootMargin = '0px'
      readonly thresholds = [0]
      disconnect() {}
      observe() {}
      takeRecords(): IntersectionObserverEntry[] { return [] }
      unobserve() {}
    })
    const { GitHubStats } = await import('./App')

    const signals: AbortSignal[] = []
    vi.stubGlobal('fetch', vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.signal) signals.push(init.signal)
      return new Promise<Response>(() => {})
    }))

    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () => {
      root.render(<GitHubStats t={withStats} />)
    })
    await act(async () => {
      root.render(<GitHubStats t={{ ...withStats, githubStats: undefined }} />)
    })
    await act(async () => {
      root.render(<GitHubStats t={withStats} />)
    })
    await act(async () => {
      root.unmount()
    })

    expect(signals).toHaveLength(4)
    expect(signals.every(signal => signal.aborted)).toBe(true)
  })
})

describe('homepage downloads', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    Reflect.deleteProperty(navigator, 'clipboard')
    document.body.innerHTML = ''
  })

  it('formats gigabyte assets without inflating the megabyte label', async () => {
    const { formatSize } = await import('./App')

    expect(formatSize(1536 * 1024 ** 2)).toBe('1.5GB')
    expect(formatSize(16 * 1024 ** 2)).toBe('16MB')
    expect(formatSize(512 * 1024)).toBe('512KB')
  })

  it('rejects an empty release response explicitly', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => [],
    } as Response))
    const { fetchLatestRelease } = await import('./App')

    await expect(fetchLatestRelease()).rejects.toThrow('No releases available')
  })

  it('drives SDK copy feedback from React state after a successful write', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    vi.stubGlobal('IntersectionObserver', class {
      readonly root = null
      readonly rootMargin = '0px'
      readonly thresholds = [0]
      disconnect() {}
      observe() {}
      takeRecords(): IntersectionObserverEntry[] { return [] }
      unobserve() {}
    })
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})))
    const { Downloads } = await import('./App')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Downloads t={getTranslation('en')} />
        </QueryClientProvider>,
      )
    })
    const button = container.querySelector<HTMLButtonElement>('[aria-label="Copy Python"]')!
    const feedback = button.querySelector('.copy-tip')!
    expect(feedback.classList.contains('opacity-0')).toBe(true)

    await act(async () => {
      button.click()
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(writeText).toHaveBeenCalledWith('pip install librefang')
    expect(feedback.classList.contains('opacity-100')).toBe(true)
    expect(feedback.classList.contains('opacity-0')).toBe(false)

    await act(async () => root.unmount())
    client.clear()
  })
})
