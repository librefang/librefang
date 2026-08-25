import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { renderMarkdown } from './minimal-markdown'

function render(md: string) {
  return renderToStaticMarkup(<>{renderMarkdown(md)}</>)
}

describe('minimal markdown links', () => {
  it.each([
    'https://example.com',
    'http://example.com',
    'mailto:hello@example.com',
    '/skills/example',
    './README.md',
    '../README.md',
    '#usage',
  ])('allows an explicit safe destination %s', (href) => {
    expect(render(`[safe](${href})`)).toContain(`href="${href}"`)
  })

  it.each([
    '//evil.example/path',
    'javascript:alert(1)',
    'data:text/html,malicious',
    'vbscript:msgbox(1)',
    'README.md',
  ])('renders an unsupported destination %s as plain text', (href) => {
    const html = render(`[unsafe](${href})`)

    expect(html).toContain('<span>unsafe</span>')
    expect(html).not.toContain('href=')
  })
})

describe('minimal markdown inline parsing', () => {
  it('renders italic text without lookbehind-dependent parsing', () => {
    expect(render('before *emphasis* after')).toContain('<em>emphasis</em>')
    expect(render('*leading* text')).toContain('<em>leading</em>')
  })
})

describe('minimal markdown block parsing', () => {
  it('requires valid delimiter cells before treating rows as a table', () => {
    expect(render('| key\n---\nafter')).not.toContain('<table')
    expect(render('| key | value |\n| --- | :---: |\n| one | two |')).toContain('<table')
  })

  it('nests README headings below the detail page section heading', () => {
    const html = render('# First\n## Second\n### Third')

    expect(html).toContain('<h3')
    expect(html).toContain('<h4')
    expect(html).toContain('<h5')
    expect(html).not.toContain('<h1')
  })
})
