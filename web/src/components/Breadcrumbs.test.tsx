/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Translation } from '../i18n'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

describe('Breadcrumbs', () => {
  it('falls back safely when optional translation blocks are absent', async () => {
    const { getBreadcrumbCopy } = await import('./Breadcrumbs')

    expect(getBreadcrumbCopy({} as Translation)).toEqual({
      label: 'Breadcrumb',
      backHome: 'Back',
    })
  })

  it('hides separators and preserves crumb nodes when entries are inserted', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const [{ default: Breadcrumbs }, { useAppStore }] = await Promise.all([
      import('./Breadcrumbs'),
      import('../store'),
    ])
    useAppStore.setState({ lang: 'en' })
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(<Breadcrumbs crumbs={[{ label: 'Alpha', href: '/alpha' }, { label: 'Current' }]} />)
    })
    const alphaLink = container.querySelector<HTMLAnchorElement>('a[href="/alpha"]')!
    expect(container.querySelector('nav')?.getAttribute('aria-label')).toBe('Breadcrumb')
    expect(Array.from(container.querySelectorAll('span[aria-hidden="true"]'))).toHaveLength(2)
    expect(alphaLink.classList.contains('truncate')).toBe(false)

    await act(async () => {
      root.render(
        <Breadcrumbs
          crumbs={[
            { label: 'Inserted', href: '/inserted' },
            { label: 'Alpha', href: '/alpha' },
            { label: 'Current' },
          ]}
        />,
      )
    })

    expect(container.querySelector('a[href="/alpha"]')).toBe(alphaLink)

    await act(async () => root.unmount())
  })
})
