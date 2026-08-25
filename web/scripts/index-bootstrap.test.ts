import { readFileSync } from 'node:fs'
import { JSDOM, type DOMWindow } from 'jsdom'
import { describe, expect, it } from 'vitest'

const INDEX_HTML = readFileSync(new URL('../index.html', import.meta.url), 'utf8')

type BootstrapWindow = DOMWindow & { __INITIAL_LANG__?: string }

function bootstrap(path: string, beforeParse?: (window: DOMWindow) => void) {
  return new JSDOM(INDEX_HTML, {
    beforeParse,
    runScripts: 'dangerously',
    url: `https://librefang.ai${path}`,
  })
}

describe('index bootstrap', () => {
  it.each([
    ['/ja', 'ja'],
    ['/ja/docs', 'ja'],
    ['/zh-TW/guide', 'zh-TW'],
    ['/jazz', 'en'],
    ['/koala', 'en'],
    ['/esports', 'en'],
    ['/zh-TWfoo', 'en'],
  ])('matches locale path segments for %s', (path, expected) => {
    const dom = bootstrap(path)
    const window = dom.window as BootstrapWindow

    expect(window.__INITIAL_LANG__).toBe(expected)
    expect(window.document.documentElement.lang).toBe(expected)
    dom.window.close()
  })

  it('loads a CJK font after detecting the locale', () => {
    const dom = bootstrap('/ja/docs')
    const window = dom.window as BootstrapWindow
    const font = window.document.querySelector<HTMLLinkElement>('link[href*="Noto+Sans+JP"]')

    expect(window.__INITIAL_LANG__).toBe('ja')
    expect(font?.rel).toBe('stylesheet')
    dom.window.close()
  })

  it('falls back to the dark theme when storage access throws', () => {
    const dom = bootstrap('/en', (window) => {
      Object.defineProperty(window, 'localStorage', {
        get() {
          throw new window.DOMException('blocked', 'SecurityError')
        },
      })
    })

    expect(dom.window.document.documentElement.classList.contains('dark')).toBe(true)
    dom.window.close()
  })

  it.each([
    ['light', 'light'],
    ['dark', 'dark'],
    ['dark mode', 'dark'],
    ['', 'dark'],
  ])('allows only supported theme value %j', (stored, expected) => {
    const dom = bootstrap('/en', (window) => window.localStorage.setItem('theme', stored))

    expect([...dom.window.document.documentElement.classList]).toEqual([expected])
    dom.window.close()
  })
})
