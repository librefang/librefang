import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Github, Twitter } from './BrandIcons'

describe('BrandIcons', () => {
  it('consumes lucide compatibility props without leaking them to SVG', () => {
    const markup = renderToStaticMarkup(<Github size={32} absoluteStrokeWidth />)

    expect(markup).toContain('width="32"')
    expect(markup).toContain('height="32"')
    expect(markup).not.toContain('absoluteStrokeWidth')
    expect(markup).not.toContain('absolute-stroke-width')
  })

  it('preserves explicit dimensions and component names through the shared factory', () => {
    const markup = renderToStaticMarkup(<Twitter width="24px" height="20px" />)

    expect(markup).toContain('width="24px"')
    expect(markup).toContain('height="20px"')
    expect(Github.displayName).toBe('Github')
    expect(Twitter.displayName).toBe('Twitter')
  })
})
