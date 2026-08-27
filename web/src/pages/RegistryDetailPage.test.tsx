/** @vitest-environment jsdom */

import { StrictMode, act } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Detail, RegistryData } from '../useRegistry'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

function registryItem(id: string, name: string, localizedName: string): Detail {
  return {
    id,
    name,
    description: `${name} description`,
    category: 'messaging',
    icon: 'message-circle',
    i18n: { ja: { name: localizedName } },
  }
}

function registryData(): RegistryData {
  return {
    hands: [],
    channels: [
      registryItem('alpha', 'Alpha', 'アルファ'),
      registryItem('beta', 'Beta', 'ベータ'),
      registryItem('gamma', 'Gamma', 'ガンマ'),
    ],
    providers: [],
    workflows: [],
    agents: [],
    plugins: [],
    skills: [],
    mcp: [],
  }
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  Reflect.deleteProperty(navigator, 'sendBeacon')
  document.body.innerHTML = ''
})

describe('RegistryDetailPage click tracking', () => {
  it('tracks a valid item once under Strict Mode and localizes adjacent names', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const sendBeacon = vi.fn(() => true)
    Object.defineProperty(navigator, 'sendBeacon', { configurable: true, value: sendBeacon })
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})))
    window.history.replaceState(null, '', '/ja/channels/beta')

    const [{ default: RegistryDetailPage }, { useAppStore }] = await Promise.all([
      import('./RegistryDetailPage'),
      import('../store'),
    ])
    useAppStore.setState({ lang: 'ja' })
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(['registry'], registryData())
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <StrictMode>
          <QueryClientProvider client={client}>
            <RegistryDetailPage category="channels" id="beta" />
          </QueryClientProvider>
        </StrictMode>,
      )
      await Promise.resolve()
    })

    expect(sendBeacon).toHaveBeenCalledTimes(1)
    const adjacentNav = Array.from(container.querySelectorAll('nav')).find((nav) =>
      nav.querySelector('a[href$="/channels/alpha"]'),
    )
    expect(adjacentNav?.textContent).toContain('アルファ')
    expect(adjacentNav?.textContent).toContain('ガンマ')
    expect(adjacentNav?.textContent).not.toContain('Alpha')
    expect(adjacentNav?.textContent).not.toContain('Gamma')

    await act(async () => root.unmount())
    client.clear()
    useAppStore.setState({ lang: 'en' })
  })

  it('does not track an unknown item', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const sendBeacon = vi.fn(() => true)
    Object.defineProperty(navigator, 'sendBeacon', { configurable: true, value: sendBeacon })
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})))
    window.history.replaceState(null, '', '/channels/missing')

    const [{ default: RegistryDetailPage }, { useAppStore }] = await Promise.all([
      import('./RegistryDetailPage'),
      import('../store'),
    ])
    useAppStore.setState({ lang: 'en' })
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(['registry'], registryData())
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <StrictMode>
          <QueryClientProvider client={client}>
            <RegistryDetailPage category="channels" id="missing" />
          </QueryClientProvider>
        </StrictMode>,
      )
      await Promise.resolve()
    })

    expect(sendBeacon).not.toHaveBeenCalled()

    await act(async () => root.unmount())
    client.clear()
  })
})
