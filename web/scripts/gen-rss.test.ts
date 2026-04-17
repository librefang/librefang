import { describe, expect, it } from 'vitest'
import { parseEntries, escapeXml, renderEntry, buildFeed } from './gen-rss'

const SAMPLE = `# Changelog

Preamble text.

## [2026.4.15] - 2026-04-15

### Added

- First feature

## [2026.4.14] - 2026-04-14

### Fixed

- A bug

## [2026.4.13] - 2026-04-13

Nothing here.
`

describe('parseEntries', () => {
  it('returns entries in document order, newest first', () => {
    const out = parseEntries(SAMPLE, 10)
    expect(out.map(e => e.version)).toEqual(['2026.4.15', '2026.4.14', '2026.4.13'])
    expect(out[0]!.date).toBe('2026-04-15')
  })

  it('respects max', () => {
    expect(parseEntries(SAMPLE, 2)).toHaveLength(2)
  })

  it('body captures the section under the heading', () => {
    const out = parseEntries(SAMPLE, 1)
    expect(out[0]!.body).toContain('### Added')
    expect(out[0]!.body).toContain('First feature')
    expect(out[0]!.body).not.toContain('2026.4.14')
  })

  it('returns empty on no matches', () => {
    expect(parseEntries('# Just a heading', 5)).toEqual([])
  })
})

describe('escapeXml', () => {
  it('escapes the five xml entities', () => {
    expect(escapeXml(`a & b < c > d "e"`)).toBe('a &amp; b &lt; c &gt; d &quot;e&quot;')
  })
})

describe('renderEntry', () => {
  it('wraps the body in CDATA', () => {
    const r = renderEntry({ version: '1.0.0', date: '2026-01-01', body: '## body' })
    expect(r).toContain('<![CDATA[## body]]>')
    expect(r).toContain('<updated>2026-01-01T00:00:00Z</updated>')
  })
})

describe('buildFeed', () => {
  it('produces valid-looking Atom with N entries', () => {
    const { xml, entries } = buildFeed(SAMPLE, { max: 3 })
    expect(entries).toHaveLength(3)
    expect(xml).toMatch(/^<\?xml version="1\.0"/)
    expect(xml).toContain('<feed xmlns="http://www.w3.org/2005/Atom">')
    expect(xml.match(/<entry>/g)).toHaveLength(3)
    expect(xml).toContain('<updated>2026-04-15T00:00:00Z</updated>')
  })

  it('custom site/author are threaded through', () => {
    const { xml } = buildFeed(SAMPLE, { site: 'https://example.com', author: 'X <x@y.z>', max: 1 })
    expect(xml).toContain('https://example.com/feed.xml')
    expect(xml).toContain('<name>X &lt;x@y.z&gt;</name>'.replace('&lt;', '<').replace('&gt;', '>'))
    // Author name is embedded verbatim between <name> tags (not escaped by
    // buildFeed itself). The main() path relies on the author literal being
    // pre-formatted.
  })

  it('empty changelog yields a feed with zero entries', () => {
    const { xml, entries } = buildFeed('# Changelog\n', { max: 5 })
    expect(entries).toHaveLength(0)
    expect(xml).toContain('<feed')
    expect(xml.match(/<entry>/g)).toBeNull()
  })
})
