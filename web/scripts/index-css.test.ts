import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const CSS = readFileSync(new URL('../src/index.css', import.meta.url), 'utf8')

describe('global accessibility styles', () => {
  it('keeps reduced-motion animations finite', () => {
    const reducedMotion = CSS.match(
      /@media \(prefers-reduced-motion: reduce\) \{([\s\S]*?)\n\}/,
    )?.[1]

    expect(reducedMotion).toBeDefined()
    expect(reducedMotion).toContain('animation-iteration-count: 1 !important')
    expect(reducedMotion).not.toContain('animation-iteration-count: infinite')
  })
})
