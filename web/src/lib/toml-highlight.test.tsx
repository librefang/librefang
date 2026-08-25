import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { highlightToml } from './toml-highlight'

describe('highlightToml', () => {
  it('keeps minus signs attached to numeric tokens in arrays', () => {
    const markup = renderToStaticMarkup(highlightToml('values = [-5, -10, 2.5]'))

    expect(markup).toContain('<span class="tk-num">-5</span>')
    expect(markup).toContain('<span class="tk-num">-10</span>')
    expect(markup).not.toContain('<span class="tk-punct">-</span>')
  })

  it('parses quoted section names containing a closing bracket', () => {
    const markup = renderToStaticMarkup(highlightToml('["my]key"] # trailing'))

    expect(markup).toContain('<span class="tk-header">&quot;my]key&quot;</span>')
    expect(markup).toContain('<span class="tk-comment"># trailing</span>')
  })

  it('keeps both closing brackets outside array-table names', () => {
    const markup = renderToStaticMarkup(highlightToml('[[array.table]]'))

    expect(markup).toContain('<span class="tk-header">array.table</span>')
    expect(markup).toContain('<span class="tk-punct">]]</span>')
  })

  it('advances safely when an unterminated token start is not consumed', () => {
    const markup = renderToStaticMarkup(highlightToml('value = "unterminated'))

    expect(markup).toContain('<span class="tk-punct">&quot;</span>')
    expect(markup).toContain('<span class="tk-punct">unterminated</span>')
  })
})
