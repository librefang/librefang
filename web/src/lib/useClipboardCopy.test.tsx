/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useClipboardCopy } from './useClipboardCopy'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('useClipboardCopy', () => {
  it('clears its reset timer during unmount', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } })
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    function Harness() {
      const { copied, copy } = useClipboardCopy()
      return <button onClick={() => copy('registry link')}>{copied ? 'copied' : 'idle'}</button>
    }

    await act(async () => root.render(<Harness />))
    await act(async () => {
      container.querySelector('button')!.click()
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(container.textContent).toBe('copied')
    expect(vi.getTimerCount()).toBe(1)

    await act(async () => root.unmount())
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps copied state false when clipboard access fails', async () => {
    vi.stubGlobal('navigator', {})
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    function Harness() {
      const { copied, copy } = useClipboardCopy()
      return <button onClick={() => copy('registry link')}>{copied ? 'copied' : 'idle'}</button>
    }

    await act(async () => root.render(<Harness />))
    await act(async () => {
      container.querySelector('button')!.click()
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(container.textContent).toBe('idle')
    await act(async () => root.unmount())
  })
})
