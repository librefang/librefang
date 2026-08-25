/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { getTranslation } from '../i18n'

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

describe('FlyDeployForm progress lifecycle', () => {
  it('does not invent completed steps and cleans pending work on unmount', async () => {
    vi.useFakeTimers()
    const intervalSpy = vi.spyOn(globalThis, 'setInterval')
    vi.stubGlobal('scrollTo', vi.fn())
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const { FlyDeployForm } = await import('./DeployPage')
    let requestSignal: AbortSignal | undefined
    vi.stubGlobal('fetch', vi.fn((_url: string, init?: RequestInit) => {
      requestSignal = init?.signal ?? undefined
      return new Promise<Response>(() => {})
    }))
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <FlyDeployForm
          onBack={() => {}}
          text={getTranslation('en').deploy!}
        />,
      )
    })
    const input = container.querySelector<HTMLInputElement>('#fly-token')!
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype, 'value',
    )!.set!
    await act(async () => {
      valueSetter.call(input, 'fo1_test_token')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    const deployButton = Array.from(container.querySelectorAll('button'))
      .find(button => button.textContent === getTranslation('en').deploy!.deployToFly)!
    await act(async () => {
      deployButton.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const progress = container.querySelector('[data-testid="deploy-progress"]')!
    expect(progress.querySelectorAll('.text-green-400')).toHaveLength(0)
    await act(async () => {
      vi.advanceTimersByTime(10_000)
    })
    expect(progress.querySelectorAll('.text-green-400')).toHaveLength(0)

    await act(async () => root.unmount())
    expect(requestSignal?.aborted).toBe(true)
    expect(intervalSpy).not.toHaveBeenCalled()
  })

  it('leaves the form idle after a successful deployment', async () => {
    vi.stubGlobal('scrollTo', vi.fn())
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const { FlyDeployForm } = await import('./DeployPage')
    let resolveRequest!: (response: Response) => void
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
      resolveRequest = resolve
    })))
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <FlyDeployForm
          onBack={() => {}}
          text={getTranslation('en').deploy!}
        />,
      )
    })
    const input = container.querySelector<HTMLInputElement>('#fly-token')!
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype, 'value',
    )!.set!
    await act(async () => {
      valueSetter.call(input, 'fo1_test_token')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    const deployButton = Array.from(container.querySelectorAll('button'))
      .find(button => button.textContent === getTranslation('en').deploy!.deployToFly)!
    await act(async () => {
      deployButton.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(container.firstElementChild?.getAttribute('aria-busy')).toBe('true')

    await act(async () => {
      resolveRequest({
        ok: true,
        json: async () => ({
          url: 'https://demo.example.com',
          dashboardUrl: 'https://fly.io/apps/demo',
          appName: 'demo',
          region: 'nrt',
        }),
      } as Response)
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(container.firstElementChild?.getAttribute('aria-busy')).toBe('false')
    expect(container.textContent).toContain(getTranslation('en').deploy!.deployed)

    await act(async () => root.unmount())
  })
})

describe('useCopy', () => {
  it('shows copied state only after the clipboard write succeeds', async () => {
    let resolveCopy!: () => void
    const writeText = vi.fn(() => new Promise<void>((resolve) => {
      resolveCopy = resolve
    }))
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { useCopy } = await import('./DeployPage')
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    function CopyHarness() {
      const { copiedKey, copy } = useCopy()
      return <button onClick={() => copy('command', 'librefang start')}>{copiedKey ?? 'idle'}</button>
    }

    await act(async () => root.render(<CopyHarness />))
    const button = container.querySelector('button')!
    await act(async () => button.click())

    expect(writeText).toHaveBeenCalledWith('librefang start')
    expect(button.textContent).toBe('idle')

    await act(async () => {
      resolveCopy()
      await Promise.resolve()
    })
    expect(button.textContent).toBe('command')

    await act(async () => root.unmount())
  })

  it('handles clipboard rejection without showing copied state', async () => {
    const writeText = vi.fn(() => Promise.reject(new Error('permission denied')))
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { useCopy } = await import('./DeployPage')
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    function CopyHarness() {
      const { copiedKey, copy } = useCopy()
      return <button onClick={() => copy('command', 'librefang start')}>{copiedKey ?? 'idle'}</button>
    }

    await act(async () => root.render(<CopyHarness />))
    const button = container.querySelector('button')!
    await act(async () => {
      button.click()
      await Promise.resolve()
    })

    expect(writeText).toHaveBeenCalledWith('librefang start')
    expect(button.textContent).toBe('idle')

    await act(async () => root.unmount())
  })

  it('handles browsers without the Clipboard API', async () => {
    vi.stubGlobal('navigator', {})
    const { useCopy } = await import('./DeployPage')
    const container = document.createElement('div')
    document.body.appendChild(container)
    const root = createRoot(container)

    function CopyHarness() {
      const { copiedKey, copy } = useCopy()
      return <button onClick={() => copy('command', 'librefang start')}>{copiedKey ?? 'idle'}</button>
    }

    await act(async () => root.render(<CopyHarness />))
    const button = container.querySelector('button')!
    await act(async () => {
      button.click()
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(button.textContent).toBe('idle')

    await act(async () => root.unmount())
  })
})
