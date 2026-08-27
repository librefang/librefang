/** @vitest-environment jsdom */

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import SiteHeader from './SiteHeader'
import { useAppStore } from '../store'

const actEnvironment = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT: boolean
}
actEnvironment.IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined

beforeEach(() => {
  document.body.innerHTML = '<div id="root"></div>'
  useAppStore.setState({ lang: 'en', theme: 'dark' })
})

afterEach(async () => {
  if (root) {
    await act(async () => root?.unmount())
    root = undefined
  }
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})

describe('SiteHeader', () => {
  it('keeps mobile selectors distinct and closes the drawer only on outside clicks', async () => {
    root = createRoot(document.querySelector('#root')!)
    await act(async () => root?.render(<SiteHeader isSubpage />))

    expect(document.querySelectorAll('[data-lang-menu]')).toHaveLength(1)
    expect(document.querySelectorAll('[data-lang-menu-mobile]')).toHaveLength(1)

    const menuButton = document.querySelector('.lucide-menu')?.closest('button')
    await act(async () => menuButton?.click())
    const drawer = document.querySelector<HTMLElement>('[data-mobile-menu]')
    expect(drawer).not.toBeNull()

    await act(async () => drawer?.dispatchEvent(new MouseEvent('click', { bubbles: true })))
    expect(document.querySelector('[data-mobile-menu]')).not.toBeNull()

    await act(async () => document.body.dispatchEvent(new MouseEvent('click', { bubbles: true })))
    expect(document.querySelector('[data-mobile-menu]')).toBeNull()
  })
})
