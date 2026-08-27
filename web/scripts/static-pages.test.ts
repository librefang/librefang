import { readFileSync } from 'node:fs'
import { JSDOM } from 'jsdom'
import { describe, expect, it } from 'vitest'

const NOT_FOUND_HTML = readFileSync(new URL('../public/404.html', import.meta.url), 'utf8')

function relativeLuminance(hex: string) {
  const channels = hex.match(/[0-9a-f]{2}/gi)
  if (!channels || channels.length !== 3) throw new Error(`Invalid color: ${hex}`)

  const [red, green, blue] = channels.map((channel) => {
    const value = Number.parseInt(channel, 16) / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })

  return 0.2126 * red + 0.7152 * green + 0.0722 * blue
}

function contrastRatio(foreground: string, background: string) {
  const lighter = Math.max(relativeLuminance(foreground), relativeLuminance(background))
  const darker = Math.min(relativeLuminance(foreground), relativeLuminance(background))
  return (lighter + 0.05) / (darker + 0.05)
}

describe('404 page', () => {
  it('exposes its content through the main landmark', () => {
    const document = new JSDOM(NOT_FOUND_HTML).window.document

    expect(document.querySelector('main.container')).not.toBeNull()
    expect(document.querySelector('main h1')?.textContent).toBe('404')
  })

  it('uses available system font stacks', () => {
    expect(NOT_FOUND_HTML).not.toContain("'Inter'")
    expect(NOT_FOUND_HTML).not.toContain("'JetBrains Mono'")
    expect(NOT_FOUND_HTML).toContain('font-family: system-ui, sans-serif')
    expect(NOT_FOUND_HTML).toContain('font-family: ui-monospace, monospace')
  })

  it('provides a visible keyboard focus style', () => {
    expect(NOT_FOUND_HTML).toMatch(/a:focus-visible\s*{[^}]*outline:/)
  })

  it('keeps paragraph contrast above the WCAG AA threshold', () => {
    expect(contrastRatio('#94a3b8', '#070b14')).toBeGreaterThanOrEqual(4.5)
  })
})
