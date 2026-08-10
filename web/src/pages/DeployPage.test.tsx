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
})
