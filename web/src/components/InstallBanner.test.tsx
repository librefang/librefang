/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import InstallBanner from './InstallBanner'
import { useAppStore } from '../store'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined

beforeEach(() => {
  document.body.innerHTML = '<div id="root"></div>'
})

afterEach(async () => {
  if (root) {
    await act(async () => root?.unmount())
    root = undefined
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

function installEvent(prompt: () => Promise<void>, outcome: 'accepted' | 'dismissed' = 'accepted') {
  const event = new Event('beforeinstallprompt', { cancelable: true })
  Object.assign(event, {
    prompt,
    userChoice: Promise.resolve({ outcome }),
  })
  return event
}

async function renderBanner() {
  useAppStore.setState({ lang: 'en' })
  root = createRoot(document.querySelector('#root')!)
  await act(async () => root?.render(<InstallBanner />))
}

describe('InstallBanner', () => {
  it('still captures and dismisses prompts when storage throws', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => { throw new Error('storage disabled') },
      setItem: () => { throw new Error('quota exceeded') },
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    await renderBanner()

    await act(async () => window.dispatchEvent(installEvent(vi.fn())))
    const buttons = document.querySelectorAll('button')
    expect(buttons).toHaveLength(2)

    await act(async () => buttons[1]?.click())
    expect(document.querySelector('button')).toBeNull()
  })

  it('consumes the prompt once and logs real prompt failures', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const failure = new Error('browser rejected prompt')
    const prompt = vi.fn().mockRejectedValue(failure)
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    await renderBanner()

    await act(async () => window.dispatchEvent(installEvent(prompt)))
    await act(async () => {
      document.querySelector('button')?.click()
      await Promise.resolve()
    })

    expect(prompt).toHaveBeenCalledOnce()
    expect(document.querySelector('button')).toBeNull()
    expect(error).toHaveBeenCalledWith('Install prompt failed:', failure)
  })

  it('closes after either defined user choice without dead branching', async () => {
    const setItem = vi.fn()
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem,
      removeItem: () => {},
      clear: () => {},
      key: () => null,
      length: 0,
    } satisfies Storage)
    const prompt = vi.fn().mockResolvedValue(undefined)
    await renderBanner()

    await act(async () => window.dispatchEvent(installEvent(prompt, 'dismissed')))
    await act(async () => {
      document.querySelector('button')?.click()
      await Promise.resolve()
    })

    expect(setItem).toHaveBeenCalledWith('librefang.install.dismissed', '1')
    expect(document.querySelector('button')).toBeNull()
  })
})
