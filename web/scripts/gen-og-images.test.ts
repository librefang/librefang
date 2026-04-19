import { describe, expect, it } from 'vitest'
import { CATEGORIES, render } from './gen-og-images'

describe('OG image generator', () => {
  it('defines exactly eight categories matching the registry', () => {
    expect(CATEGORIES).toHaveLength(8)
    const slugs = CATEGORIES.map(c => c.slug).sort()
    expect(slugs).toEqual([
      'agents', 'channels', 'hands', 'mcp',
      'plugins', 'providers', 'skills', 'workflows',
    ])
  })

  it('each category has a unique accent colour', () => {
    const accents = CATEGORIES.map(c => c.accent)
    expect(new Set(accents).size).toBe(accents.length)
  })

  it('each category has a title and subtitle', () => {
    for (const c of CATEGORIES) {
      expect(c.title.length).toBeGreaterThan(0)
      expect(c.subtitle.length).toBeGreaterThan(0)
      expect(c.icon.length).toBeGreaterThan(0)
    }
  })

  it('render produces valid-looking SVG with the category accent colour', () => {
    for (const c of CATEGORIES) {
      const svg = render(c)
      expect(svg).toMatch(/^<svg\b/)
      expect(svg).toContain('</svg>')
      expect(svg).toContain(c.title)
      expect(svg).toContain(c.subtitle)
      expect(svg).toContain(c.accent)
      // Must declare the OG image size so link unfurls get the right crop.
      expect(svg).toContain('width="1200"')
      expect(svg).toContain('height="630"')
    }
  })
})
