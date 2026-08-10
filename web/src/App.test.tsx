// @vitest-environment jsdom
// @vitest-environment-options { "url": "https://librefang.local/" }

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Translation } from './i18n'

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
