/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  buildLocalizedPath,
  detectLangFromPath,
  readStoredTheme,
  stripLocalePrefix,
  useAppStore,
} from './store'

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.documentElement.classList.remove('dark', 'light')
  document.head.querySelectorAll('link[href*="fonts.googleapis.com"]').forEach(link => link.remove())
  window.history.replaceState(null, '', '/')
  useAppStore.setState({ lang: 'en', theme: 'dark' })
})

describe('app store contracts', () => {
  it('detects supported locales from one ordered boundary-aware list', () => {
    expect(detectLangFromPath('/zh-TW/skills/example')).toBe('zh-TW')
    expect(detectLangFromPath('/zh/skills/example')).toBe('zh')
    expect(detectLangFromPath('/zh-not-a-locale')).toBe('en')
    expect(detectLangFromPath('/', 'ja')).toBe('ja')
    expect(detectLangFromPath('/', 'invalid')).toBe('en')
  })

  it('builds localized paths without nested URL branching', () => {
    expect(stripLocalePrefix('/de/skills/example')).toBe('/skills/example')
    expect(buildLocalizedPath('ja', '/de/skills/example')).toBe('/ja/skills/example')
    expect(buildLocalizedPath('ja', '/')).toBe('/ja')
    expect(buildLocalizedPath('en', '/uk')).toBe('/')
  })

  it('accepts only supported stored theme values and survives storage errors', () => {
    expect(readStoredTheme({ getItem: () => 'light' })).toBe('light')
    expect(readStoredTheme({ getItem: () => 'system' })).toBe('dark')
    expect(readStoredTheme({ getItem: () => { throw new Error('blocked') } })).toBe('dark')
  })

  it('switches locale with query and hash preservation and no server-side effects', () => {
    window.history.replaceState(null, '', '/de/skills/example?tab=readme#usage')
    useAppStore.setState({ lang: 'de' })

    useAppStore.getState().switchLang('ja')

    expect(window.location.pathname).toBe('/ja/skills/example')
    expect(window.location.search).toBe('?tab=readme')
    expect(window.location.hash).toBe('#usage')
    expect(document.documentElement.lang).toBe('ja')
    expect(document.head.querySelector('link[href*="Noto+Sans+JP"]')).not.toBeNull()

    const activeLang = useAppStore.getState().lang
    vi.stubGlobal('window', undefined)
    useAppStore.getState().switchLang('ko')
    expect(useAppStore.getState().lang).toBe(activeLang)
  })

  it('keeps side effects outside the pure Zustand state update', () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem')
    useAppStore.setState({ theme: 'dark' })

    useAppStore.getState().toggleTheme()

    expect(useAppStore.getState().theme).toBe('light')
    expect(setItem).toHaveBeenCalledWith('theme', 'light')
    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })
})
