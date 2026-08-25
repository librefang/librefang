/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import ErrorBoundary from './ErrorBoundary'
import { useAppStore } from '../store'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined

afterEach(async () => {
  if (root) {
    await act(async () => root?.unmount())
    root = undefined
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

function ThrowingChild(): never {
  throw new Error('secret-token=should-stay-local')
}

describe('ErrorBoundary', () => {
  it('renders recovery UI without sending exception details externally', async () => {
    const fetch = vi.fn()
    const sendBeacon = vi.fn()
    vi.stubGlobal('fetch', fetch)
    vi.stubGlobal('navigator', { sendBeacon })
    vi.spyOn(console, 'error').mockImplementation(() => {})
    useAppStore.setState({ lang: 'en' })
    document.body.innerHTML = '<div id="root"></div>'
    root = createRoot(document.querySelector('#root')!)

    await act(async () => root?.render(
      <ErrorBoundary>
        <ThrowingChild />
      </ErrorBoundary>,
    ))

    expect(document.body.textContent).toContain('secret-token=should-stay-local')
    expect(fetch).not.toHaveBeenCalled()
    expect(sendBeacon).not.toHaveBeenCalled()
  })
})
